//! 미디어 오퍼(`0x1c`)와 그 응답입니다.
//!
//! - 오퍼는 오디오와 비디오 각각의 바이너리 plist이며, 그 안에 zlib으로 압축한 protobuf(MediaBlob)가 있습니다.
//! - 비디오는 기본으로 코덱 뱅크 123만 넣고 타일 1개를 요청해 H.264 4:2:0 단일 스트림을 받습니다. 뱅크 100을 넣으면 서버가 HEVC 4:4:4를 보내는데, 일반적인 모바일 하드웨어 디코더는 이를 풀지 못합니다.
//! - 오디오 오퍼는 생략할 수 없습니다. 오디오를 끌 때는 요청 비트레이트를 낮춰 서버가 오디오를 보내지 않게 합니다.
//! - SRTP 마스터 키는 클라이언트가 만들어 오퍼에 실어 보냅니다. 제어 채널이 뚫리면 미디어도 함께 뚫립니다.
use crate::{Bank, Codec, Features};
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

/// SRTP 마스터 키(32바이트)와 마스터 솔트(14바이트)를 이은 키 블롭의 길이
pub const KEY_BLOB_BYTES: usize = 46;

// 플래그: 두 스트림의 60fps와, 커서를 영상에 넣지 않는 비트입니다. 서버는 이 필드를 빅엔디언으로 읽습니다.
const FLAGS: u32 = 0x01 | 0x02 | 0x04;
const VERSION: u16 = 3;

const MODE_VIDEO: i64 = 7;
const MODE_AUDIO: i64 = 8;

// H.264를 받을 때는 LTR을 끄고 기능 문자열에서도 뺍니다. 이름은 HEVC용처럼 보이지만 서버는 이 뱅크로 H.264를 보냅니다.
const H264_BANK: u32 = 123;
const H264_FEATURES: &str = "FLS;MS:-1;LF:-1;CABAC;POS:0;EOD:1;HTS:2;RR:3;AR:16/9,5/8;XR:16/9,5/8;";

// HEVC 4:4:4는 Apple 클라이언트가 LTR과 피드백을 켜고 보내는 값을 그대로 씁니다.
const HEVC_BANK: u32 = 100;
const HEVC_FEATURES: &str = "FLS;LF:-1;POS:5;EOD:1;HTS:2;RR:3;POSE:4;AR:16/9,5/8;XR:16/9,5/8;";

// 요청 비트레이트는 비례 값이 아니라 계층 선택자입니다. 약 5000 미만이면 서버가 오디오를 보내지 않습니다.
const AUDIO_ON: u32 = 24191;
const AUDIO_OFF: u32 = 1000;

// 서버가 기대하는 오디오 계층 표입니다. 비디오 오퍼에도 똑같이 넣습니다.
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

// 응답은 프레임버퍼 갱신 본문 안의 바이너리 plist입니다. 압축을 풀었을 때의 크기 한도를 둡니다.
const PLIST_MARKER: &[u8] = b"bplist00";
const MAX_ANSWER_BYTES: u64 = 1 << 20;

/// 한 세션의 미디어 오퍼입니다. 재전송과 해상도 변경 때 같은 오퍼를 다시 보냅니다.
pub struct Offer {
    session_id: [u8; 16],
    /// SRTP 키 블롭
    pub keys: MediaKeys,
    /// 클라이언트의 오디오 SSRC. 오디오 keepalive의 SSRC입니다.
    pub audio_ssrc: u32,
    /// 클라이언트의 비디오 SSRC. 클라이언트가 보내는 RTCP의 송신자 SSRC입니다.
    pub video_ssrc: u32,
    audio: bool,
    codec: Codec,
}

/// 오퍼에 싣는 SRTP 키 블롭 네 개입니다.
#[derive(Clone)]
pub struct MediaKeys {
    /// 클라이언트가 보내는 오디오 keepalive
    pub audio_send: [u8; KEY_BLOB_BYTES],
    /// 서버가 보내는 오디오
    pub audio_receive: [u8; KEY_BLOB_BYTES],
    /// 클라이언트가 보내는 RTCP
    pub video_send: [u8; KEY_BLOB_BYTES],
    /// 서버가 보내는 비디오와 RTCP
    pub video_receive: [u8; KEY_BLOB_BYTES],
}

/// 서버가 돌려준 비디오 협상 결과입니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Answer {
    /// 캔버스 크기(픽셀). 서버의 인코더가 아직 준비되지 않으면 없습니다.
    pub size: Option<(u32, u32)>,
}

impl Offer {
    /// 새 키와 SSRC로 오퍼를 만듭니다.
    pub fn new(audio: bool, codec: Codec) -> Self {
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

    /// `0x1c` 메시지를 만듭니다.
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

        // H.264는 LTR과 피드백을 끄고, HEVC는 켭니다.
        let request = match self.codec {
            Codec::H264 => Bank {
                id: H264_BANK,
                field_4: 1,
                features: Features::H264,
                tiles: 1,
                ltr: false,
            },
            Codec::Hevc => Bank {
                id: HEVC_BANK,
                field_4: 14,
                features: Features::Hevc,
                tiles: 1,
                ltr: true,
            },
            Codec::Custom(bank) => bank,
        };
        let (kinds, features): (&[u32], _) = match request.features {
            Features::H264 => (&[1, 2, 1, 2], H264_FEATURES),
            Features::Hevc => (&[1, 2], HEVC_FEATURES),
        };
        let bank = CodecBank {
            id: Some(request.id),
            resolutions: kinds.iter().copied().map(resolution).collect(),
            features: Some(features.into()),
            field_4: Some(request.field_4),
        };
        let (tiles, ltr) = (request.tiles, u32::from(request.ltr));

        MediaBlob {
            video: Some(VideoStream {
                ssrc: Some(self.video_ssrc),
                feedback: Some(ltr),
                banks: vec![bank],
                tiles: Some(tiles),
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
            .expect("Vec에 쓰는 압축은 실패하지 않는다");
        let compressed = encoder.finish().expect("Vec에 쓰는 압축은 실패하지 않는다");

        // 끝점 정보는 서버가 참고만 하므로 값이 자유롭습니다.
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
            .expect("Vec에 쓰는 plist 직렬화는 실패하지 않는다");

        bytes
    }
}

impl Answer {
    /// 미디어 스트림 사각형의 본문에서 비디오 응답을 찾습니다. 응답이 없으면 `None`입니다.
    pub fn find(body: &[u8]) -> Option<Self> {
        // 본문 안에 plist가 여러 개이거나 plist 뒤에 다른 바이트가 이어질 수 있습니다.
        // 바이너리 plist는 끝의 32바이트 트레일러로 해석하므로, 표식마다 트레일러로 보이는 끝 위치를 찾아 시도합니다.
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
            size: video
                .width
                .zip(video.height)
                .filter(|&(width, height)| width > 0 && height > 0),
        })
    }
}

// 바이너리 plist 트레일러는 0 여섯 바이트, 오프셋 크기, 참조 크기, 객체 수, 최상위 객체, 오프셋 표 위치(각 8바이트)입니다.
fn trailer_ends(bytes: &[u8]) -> impl Iterator<Item = usize> + '_ {
    (40..=bytes.len()).filter(move |&end| {
        let trailer = &bytes[end - 32..end];
        let offsets = u64::from_be_bytes(trailer[24..32].try_into().expect("8바이트다"));
        trailer[..6] == [0; 6]
            && (1..=8).contains(&trailer[6])
            && (1..=8).contains(&trailer[7])
            && offsets < (end - 32) as u64
    })
}

// 오퍼의 길이 필드는 `u16`입니다. 오퍼는 이 크레이트가 정한 고정 내용이라 넘치지 않습니다.
fn length(len: usize) -> u16 {
    u16::try_from(len).expect("오퍼는 64KiB보다 작다")
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

// 대문자 UUID 문자열(8-4-4-4-12)입니다.
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
    use crate::Codec;
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
        let offer = Offer::new(false, Codec::H264);
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
        let offer = Offer::new(false, Codec::H264);
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
    fn hevc_offer_requests_a_single_tile_with_ltr() {
        let offer = Offer::new(false, Codec::Hevc);
        let message = offer.encode();
        let audio_len = u16::from_be_bytes([message[0x0a], message[0x0b]]) as usize;
        let video = blob(&message[0x80 + audio_len + 92..]).video.unwrap();

        assert_eq!((video.tiles, video.ltr, video.feedback), (Some(1), Some(1), Some(1)));
        assert_eq!(video.banks.len(), 1);
        assert_eq!((video.banks[0].id, video.banks[0].field_4), (Some(100), Some(14)));
        assert_eq!(video.banks[0].resolutions.len(), 2);
        assert_eq!(video.banks[0].features.as_deref(), Some(HEVC_FEATURES));
    }

    #[test]
    fn audio_off_requests_a_bitrate_below_the_threshold() {
        let message = Offer::new(false, Codec::H264).encode();
        let audio = audio_blob(&message).audio.unwrap();
        assert!(audio.bitrate.unwrap() < 5000);

        let message = Offer::new(true, Codec::H264).encode();
        assert_eq!(audio_blob(&message).audio.unwrap().bitrate, Some(24191));
    }

    #[test]
    fn zero_valued_fields_are_still_encoded() {
        let message = Offer::new(false, Codec::H264).encode();
        let encoded = audio_blob(&message).encode_to_vec();

        // 필드 8과 16(값 0)이 각각 태그 바이트 0x40, 0x80 0x01로 들어 있어야 합니다.
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

        assert_eq!(
            Answer::find(&body),
            Some(Answer {
                size: Some((2560, 1600)),
            })
        );
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

        assert_eq!(Answer::find(&body).unwrap().size, None);
        assert_eq!(Answer::find(b"no plist here"), None);
    }

    #[test]
    fn call_id_is_an_uppercase_uuid() {
        assert_eq!(call_id(&[0xab; 16]), "ABABABAB-ABAB-ABAB-ABAB-ABABABABABAB");
    }
}
