//! The media offer (`0x1c`) and its answer.
//!
//! - The offer is one binary plist each for audio and video, holding a zlib-compressed protobuf (MediaBlob).
//! - For H.264 the video offer lists only codec bank 123 and requests one tile, which yields a single 4:2:0 stream.
//!   For HEVC it lists bank 100, and the server sends 4:4:4 in four bands, which many hardware decoders can't decode.
//! - The audio offer can't be omitted. To turn audio off, a low requested bitrate stops the server from sending it.
//! - The client generates the SRTP master keys and sends them in the offer, so compromising the control channel also
//!   compromises the media.
use crate::media::VideoCodec;
use blob::{AnswerBlob, AudioStream, AudioTier, CodecBank, EndpointInfo, MediaBlob, Resolution, VideoStream};
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use prost::Message;
use rand::RngCore;
use rand::rngs::OsRng;
use std::io::{Read, Write};
use std::time::{SystemTime, UNIX_EPOCH};

mod blob;

/// Length of a key blob: the SRTP master key (32 bytes) followed by the master salt (14 bytes)
pub const KEY_BLOB_BYTES: usize = 46;

// Flags: 60 fps for both streams, and keep the cursor out of the video. The server reads this field as big-endian.
const FLAGS: u32 = 0x01 | 0x02 | 0x04;
const VERSION: u16 = 3;

const MODE_VIDEO: i64 = 7;
const MODE_AUDIO: i64 = 8;

// For H.264, LTR is off and left out of the feature string.
const H264_BANK: u32 = 123;
const H264_FEATURES: &str = "FLS;MS:-1;LF:-1;CABAC;POS:0;EOD:1;HTS:2;RR:3;AR:16/9,5/8;XR:16/9,5/8;";

// HEVC 4:4:4 uses the values Apple's client sends, with LTR and feedback on and four tiles.
const HEVC_BANK: u32 = 100;
const HEVC_FEATURES: &str = "FLS;LF:-1;POS:5;EOD:1;HTS:2;RR:3;POSE:4;AR:16/9,5/8;XR:16/9,5/8;";

// The requested bitrate is a tier selector, not a proportional value. Below about 5000 the server sends no audio.
const AUDIO_ON: u32 = 24191;
const AUDIO_OFF: u32 = 1000;

// Audio tier table the server expects. The video offer carries the same table.
const AUDIO_TIERS: [(u32, u32, Option<u32>); 10] = [
    (0, 40_000_000, Some(12_288)),
    (0, 6_000_000, Some(131_072)),
    (4074, 0, Some(16_384)),
    (16, 4100, None),
    (0, 75_000_000, Some(524_288)),
    (0, 20_000_000, Some(98_304)),
    (4, 6500, None),
    (0, 60_000_000, Some(262_144)),
    (1, 299, None),
    (0, 100_000_000, Some(1_048_576)),
];

const CLIENT_NAME: &str = "Naru";

// The answer is a binary plist inside a framebuffer update body. Its decompressed size is capped.
const PLIST_MARKER: &[u8] = b"bplist00";
const MAX_ANSWER_BYTES: u64 = 1 << 20;

/// Media offer for one session. The same offer is resent on retransmission and resolution changes.
pub struct Offer {
    session_id: [u8; 16],
    /// SRTP key blobs
    pub keys: MediaKeys,
    /// Client audio SSRC, used as the sender SSRC of the client's audio RTCP
    pub audio_ssrc: u32,
    /// Client video SSRC, used as the sender SSRC of the client's RTCP
    pub video_ssrc: u32,
    audio: bool,
    codec: VideoCodec,
}

/// The four SRTP key blobs carried in the offer.
pub struct MediaKeys {
    /// Audio RTCP sent by the client
    pub audio_send: [u8; KEY_BLOB_BYTES],
    /// Audio sent by the server
    pub audio_receive: [u8; KEY_BLOB_BYTES],
    /// RTCP sent by the client
    pub video_send: [u8; KEY_BLOB_BYTES],
    /// Video and RTCP sent by the server
    pub video_receive: [u8; KEY_BLOB_BYTES],
}

/// Video negotiation result returned by the server.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Answer {
    /// Whether the server's encoder is ready, which it shows by naming the canvas size
    pub ready: bool,
}

impl Offer {
    /// Builds an offer with fresh keys and SSRCs.
    pub fn new(audio: bool, codec: VideoCodec) -> Self {
        Self {
            keys: MediaKeys {
                audio_send: random(),
                audio_receive: random(),
                video_send: random(),
                video_receive: random(),
            },
            session_id: random(),
            audio_ssrc: OsRng.next_u32(),
            video_ssrc: OsRng.next_u32(),
            audio,
            codec,
        }
    }

    /// Builds the `0x1c` message.
    pub fn encode(&self) -> Vec<u8> {
        let audio = self.plist(MODE_AUDIO, self.audio_blob());
        let video = self.plist(MODE_VIDEO, self.video_blob());

        let mut message = vec![0x1c, 0];
        message.extend_from_slice(&length(audio.len() + video.len() + 0xd8).to_be_bytes());
        message.extend_from_slice(&VERSION.to_be_bytes());
        message.extend_from_slice(&FLAGS.to_be_bytes());
        message.extend_from_slice(&length(audio.len()).to_be_bytes());
        message.extend_from_slice(&length(video.len()).to_be_bytes());
        message.extend_from_slice(&[0; 6]);
        message.extend_from_slice(&self.session_id);
        message.extend_from_slice(&self.keys.audio_send);
        message.extend_from_slice(&self.keys.audio_receive);
        message.extend_from_slice(&audio);
        message.extend_from_slice(&self.keys.video_send);
        message.extend_from_slice(&self.keys.video_receive);
        message.extend_from_slice(&video);
        message
    }

    fn audio_blob(&self) -> MediaBlob {
        let bitrate = if self.audio { AUDIO_ON } else { AUDIO_OFF };

        MediaBlob {
            audio: Some(AudioStream {
                ssrc: Some(self.audio_ssrc),
                field_2: Some(0),
                field_3: Some(0),
                bitrate: Some(bitrate),
                field_5: Some(0),
                field_6: Some(0),
            }),
            ..common_blob()
        }
    }

    fn video_blob(&self) -> MediaBlob {
        let resolution = |kind| Resolution {
            field_1: Some(1),
            kind: Some(kind),
            field_3: Some(50115),
            field_4: Some(0),
        };

        // LTR and feedback are off for H.264 and on for HEVC.
        let (id, kinds, features, field_4, ltr): (_, &[u32], _, _, _) = match self.codec {
            VideoCodec::H264 => (H264_BANK, &[1, 2, 1, 2], H264_FEATURES, 1, 0),
            VideoCodec::Hevc => (HEVC_BANK, &[1, 2], HEVC_FEATURES, 14, 1),
        };
        let bank = CodecBank {
            id: Some(id),
            resolutions: kinds.iter().copied().map(resolution).collect(),
            features: Some(features.into()),
            field_4: Some(field_4),
        };

        MediaBlob {
            video: Some(VideoStream {
                ssrc: Some(self.video_ssrc),
                feedback: Some(ltr),
                banks: vec![bank],
                tiles: Some(u32::from(self.codec.bands())),
                ltr: Some(ltr),
                field_8: Some(63),
                field_9: Some(1),
                field_12: Some(1),
            }),
            ..common_blob()
        }
    }

    fn plist(&self, mode: i64, blob: MediaBlob) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&blob.encode_to_vec())
            .expect("compressing into a Vec cannot fail");
        let compressed = encoder.finish().expect("compressing into a Vec cannot fail");

        // The server treats the endpoint info as informational only, so any values work.
        let endpoint = EndpointInfo {
            field_1: Some(0),
            field_2: Some(1),
            model: Some(CLIENT_NAME.into()),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            build: Some(env!("CARGO_PKG_VERSION").into()),
        };

        let mut dict = plist::Dictionary::new();
        dict.insert("avcMediaStreamNegotiatorMode".into(), mode.into());
        dict.insert(
            "avcMediaStreamNegotiatorMediaBlob".into(),
            plist::Value::Data(compressed),
        );
        dict.insert("avcMediaStreamOptionCallID".into(), call_id(&self.session_id).into());

        dict.insert(
            "avcMediaStreamOptionRemoteEndpointInfo".into(),
            plist::Value::Data(endpoint.encode_to_vec()),
        );

        let mut bytes = Vec::new();

        plist::Value::Dictionary(dict)
            .to_writer_binary(&mut bytes)
            .expect("serializing a plist into a Vec cannot fail");

        bytes
    }
}

impl Answer {
    /// Finds the video answer in a media stream rectangle's body. Returns `None` if there is no answer.
    pub fn find(body: &[u8]) -> Option<Self> {
        // The body may hold several plists, or other bytes may follow a plist. A binary plist is parsed from its
        // trailing 32-byte trailer, so for each marker every end position that looks like a trailer is tried.
        (0..body.len())
            .filter(|&start| body[start..].starts_with(PLIST_MARKER))
            .flat_map(|start| trailer_ends(&body[start..]).map(move |end| &body[start..start + end]))
            .find_map(Self::parse)
    }

    fn parse(bytes: &[u8]) -> Option<Self> {
        let value: plist::Value = plist::from_bytes(bytes).ok()?;
        let blob = value
            .as_dictionary()?
            .get("avcMediaStreamNegotiatorMediaBlob")?
            .as_data()?;

        let mut decoded = Vec::new();
        ZlibDecoder::new(blob)
            .take(MAX_ANSWER_BYTES)
            .read_to_end(&mut decoded)
            .ok()?;

        let video = AnswerBlob::decode(decoded.as_slice()).ok()?.video?;

        Some(Self {
            ready: video.width.unwrap_or(0) > 0 && video.height.unwrap_or(0) > 0,
        })
    }
}

// End positions where the preceding 32 bytes look like a binary plist trailer.
fn trailer_ends(bytes: &[u8]) -> impl Iterator<Item = usize> + '_ {
    (40..=bytes.len()).filter(move |&end| {
        let trailer = &bytes[end - 32..end];
        let offsets = u64::from_be_bytes(trailer[24..32].try_into().expect("8 bytes"));
        trailer[..6] == [0; 6]
            && (1..=8).contains(&trailer[6])
            && (1..=8).contains(&trailer[7])
            && offsets < (end - 32) as u64
    })
}

// Offer length fields are `u16`. The offer's contents are fixed by this crate, so they never overflow.
fn length(len: usize) -> u16 {
    u16::try_from(len).expect("the offer is smaller than 64 KiB")
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

fn common_blob() -> MediaBlob {
    let timestamp_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or_default();

    let tier = |(field_1, field_2, field_3)| AudioTier {
        field_1: Some(field_1),
        field_2: Some(field_2),
        field_3,
    };

    MediaBlob {
        field_1: Some(1),
        field_2: Some(1),
        audio: None,
        video: None,
        client_name: Some(CLIENT_NAME.into()),
        field_8: Some(0),
        audio_tiers: AUDIO_TIERS.into_iter().map(tier).collect(),
        timestamp_ns: Some(timestamp_ns),
        field_14: Some(2),
        field_16: Some(0),
        field_18: Some(1),
    }
}

// Uppercase UUID string (8-4-4-4-12).
fn call_id(id: &[u8; 16]) -> String {
    let hex: String = id.iter().map(|byte| format!("{byte:02X}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

#[cfg(test)]
mod tests {
    use super::blob::{AnswerBlob, MediaBlob, VideoAnswer};
    use super::{Answer, H264_FEATURES, HEVC_FEATURES, Offer, call_id};
    use crate::media::VideoCodec;
    use flate2::Compression;
    use flate2::read::ZlibDecoder;
    use flate2::write::ZlibEncoder;
    use prost::Message;
    use std::io::{Read, Write};

    fn blob(plist_bytes: &[u8]) -> MediaBlob {
        let value: plist::Value = plist::from_bytes(plist_bytes).unwrap();
        let data = value.as_dictionary().unwrap()["avcMediaStreamNegotiatorMediaBlob"]
            .as_data()
            .unwrap();

        let mut decoded = Vec::new();
        ZlibDecoder::new(data).read_to_end(&mut decoded).unwrap();
        MediaBlob::decode(decoded.as_slice()).unwrap()
    }

    fn audio_blob(message: &[u8]) -> MediaBlob {
        let audio_len = u16::from_be_bytes([message[0x0a], message[0x0b]]) as usize;
        blob(&message[0x80..0x80 + audio_len])
    }

    #[test]
    fn layout_places_keys_and_offers_at_their_offsets() {
        let offer = Offer::new(false, VideoCodec::H264);
        let message = offer.encode();

        let audio_len = u16::from_be_bytes([message[0x0a], message[0x0b]]) as usize;
        let video_len = u16::from_be_bytes([message[0x0c], message[0x0d]]) as usize;

        assert_eq!(message.len(), audio_len + video_len + 0xdc);
        assert_eq!(u16::from_be_bytes([message[2], message[3]]) as usize, message.len() - 4);
        assert_eq!(&message[4..10], [0, 3, 0, 0, 0, 7]);
        assert_eq!(&message[0x0e..0x14], [0; 6]);
        assert_eq!(message[0x24..0x52], offer.keys.audio_send);
        assert_eq!(message[0x52..0x80], offer.keys.audio_receive);

        let video_keys = 0x80 + audio_len;
        assert_eq!(message[video_keys..video_keys + 46], offer.keys.video_send);
        assert_eq!(message[video_keys + 46..video_keys + 92], offer.keys.video_receive);
        assert_eq!(&message[video_keys + 92..video_keys + 100], b"bplist00");
    }

    #[test]
    fn video_offer_requests_single_tile_h264_without_ltr() {
        let offer = Offer::new(false, VideoCodec::H264);
        let message = offer.encode();
        let audio_len = u16::from_be_bytes([message[0x0a], message[0x0b]]) as usize;
        let video = blob(&message[0x80 + audio_len + 92..]).video.unwrap();

        assert_eq!(video.ssrc, Some(offer.video_ssrc));
        assert_eq!((video.tiles, video.ltr, video.feedback), (Some(1), Some(0), Some(0)));
        assert_eq!(video.banks.len(), 1);
        assert_eq!(video.banks[0].id, Some(123));
        assert_eq!(video.banks[0].features.as_deref(), Some(H264_FEATURES));
        assert!(!H264_FEATURES.contains("LTR;"));
    }

    #[test]
    fn hevc_offer_requests_four_tiles_with_ltr() {
        let offer = Offer::new(false, VideoCodec::Hevc);
        let message = offer.encode();
        let audio_len = u16::from_be_bytes([message[0x0a], message[0x0b]]) as usize;
        let video = blob(&message[0x80 + audio_len + 92..]).video.unwrap();

        assert_eq!((video.tiles, video.ltr, video.feedback), (Some(4), Some(1), Some(1)));
        assert_eq!(video.banks.len(), 1);
        assert_eq!((video.banks[0].id, video.banks[0].field_4), (Some(100), Some(14)));
        assert_eq!(video.banks[0].resolutions.len(), 2);
        assert_eq!(video.banks[0].features.as_deref(), Some(HEVC_FEATURES));
    }

    #[test]
    fn audio_off_requests_a_bitrate_below_the_threshold() {
        let message = Offer::new(false, VideoCodec::H264).encode();
        let audio = audio_blob(&message).audio.unwrap();
        assert!(audio.bitrate.unwrap() < 5000);

        let message = Offer::new(true, VideoCodec::H264).encode();
        assert_eq!(audio_blob(&message).audio.unwrap().bitrate, Some(24191));
    }

    #[test]
    fn zero_valued_fields_are_still_encoded() {
        let message = Offer::new(false, VideoCodec::H264).encode();
        let encoded = audio_blob(&message).encode_to_vec();

        // Fields 8 and 16 (value 0) must be present with tag bytes 0x40 and 0x80 0x01.
        assert!(encoded.windows(2).any(|pair| pair == [0x40, 0]));
        assert!(encoded.windows(3).any(|triple| triple == [0x80, 0x01, 0]));
    }

    #[test]
    fn answer_is_found_inside_a_media_stream_body() {
        let answer = AnswerBlob {
            video: Some(VideoAnswer {
                width: Some(2560),
                height: Some(1600),
            }),
        };

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&answer.encode_to_vec()).unwrap();

        let mut dict = plist::Dictionary::new();
        dict.insert(
            "avcMediaStreamNegotiatorMediaBlob".into(),
            plist::Value::Data(encoder.finish().unwrap()),
        );

        let mut body = b"\x00\x01prefix".to_vec();
        plist::Value::Dictionary(dict).to_writer_binary(&mut body).unwrap();
        body.extend_from_slice(b"trailing bytes after the plist");

        assert_eq!(Answer::find(&body), Some(Answer { ready: true }));
    }

    #[test]
    fn answer_without_size_means_the_encoder_is_not_ready() {
        let answer = AnswerBlob {
            video: Some(VideoAnswer::default()),
        };

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&answer.encode_to_vec()).unwrap();

        let mut dict = plist::Dictionary::new();
        dict.insert(
            "avcMediaStreamNegotiatorMediaBlob".into(),
            plist::Value::Data(encoder.finish().unwrap()),
        );

        let mut body = Vec::new();
        plist::Value::Dictionary(dict).to_writer_binary(&mut body).unwrap();

        assert_eq!(Answer::find(&body), Some(Answer { ready: false }));
        assert_eq!(Answer::find(b"no plist here"), None);
    }

    #[test]
    fn call_id_is_an_uppercase_uuid() {
        assert_eq!(call_id(&[0xab; 16]), "ABABABAB-ABAB-ABAB-ABAB-ABABABABABAB");
    }
}
