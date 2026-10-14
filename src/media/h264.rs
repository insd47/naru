//! H.264 RTP payload (RFC 6184, non-interleaved mode) and decoder config.
//!
//! - The server doesn't send SPS and PPS as separate NAL units; it sends them as an ISO BMFF `avcC` box in a config
//!   packet whose first byte is `0x92`. That byte has the forbidden bit set and isn't a valid NAL header, so it is
//!   checked before the NAL type.
//! - A keyframe can arrive as an I slice inside a type 1 NAL unit instead of an IDR (type 5).
use super::bits::Bits;
use super::unit::{DecoderConfig, VideoCodec};
use crate::error::Result;
use crate::reader::Reader;

/// First byte of a decoder config packet
pub const CONFIG_MARKER: u8 = 0x92;

const NAL_SLICE: u8 = 1;
const NAL_IDR: u8 = 5;
/// SPS NAL type
pub const NAL_SPS: u8 = 7;
const NAL_STAP_A: u8 = 24;
const NAL_FU_A: u8 = 28;
const NAL_FU_B: u8 = 29;

impl DecoderConfig {
    /// Builds a decoder config from the `avcC` box in an H.264 config packet's payload. Returns `None` if it isn't a
    /// config packet or is malformed.
    pub fn from_h264(payload: &[u8]) -> Option<Self> {
        if payload.first() != Some(&CONFIG_MARKER) {
            return None;
        }

        // Finds the `avcC` box inside the `avc1` sample entry. The box size is the 4 bytes before the type.
        let search_from = find(payload, b"avc1").unwrap_or(0);
        let at = search_from + find(&payload[search_from..], b"avcC")?;
        let size = u32::from_be_bytes(payload.get(at.checked_sub(4)?..at)?.try_into().ok()?) as usize;
        let record = payload.get(at + 4..at - 4 + size)?;

        Self::read(&mut Reader::new(record, "avcC"))
            .ok()
            .filter(|config| !config.sps.is_empty() && !config.pps.is_empty())
    }

    fn read(reader: &mut Reader) -> Result<Self> {
        reader.skip(1)?;
        let profile = reader.u8()?;
        let compatibility = reader.u8()?;
        let level = reader.u8()?;
        reader.skip(1)?;

        let sps_count = reader.u8()? & 0x1f;
        let sps = units(reader, sps_count)?;
        let pps_count = reader.u8()?;
        let pps = units(reader, pps_count)?;

        Ok(Self {
            codec: VideoCodec::H264,
            bands: VideoCodec::H264.bands(),
            profile,
            compatibility,
            level,
            vps: Vec::new(),
            sps,
            pps,
        })
    }
}

/// Turns one access unit's payloads back into a list of NAL units. Returns `None` if a fragment is missing or
/// malformed.
pub fn depacketize<'a>(payloads: impl IntoIterator<Item = &'a [u8]>) -> Option<Vec<Vec<u8>>> {
    let mut nals = Vec::new();
    let mut fragment: Option<Vec<u8>> = None;

    for payload in payloads {
        let indicator = *payload.first()?;

        match indicator & 0x1f {
            1..=23 => {
                fragment.is_none().then_some(())?;
                nals.push(payload.to_vec());
            }
            NAL_STAP_A => {
                fragment.is_none().then_some(())?;
                let mut reader = Reader::new(&payload[1..], "STAP-A");

                while reader.remaining() > 0 {
                    let len = reader.u16().ok()? as usize;
                    nals.push(reader.bytes(len).ok()?.to_vec());
                }
            }
            kind @ (NAL_FU_A | NAL_FU_B) => {
                let header = *payload.get(1)?;
                let body = payload.get(if kind == NAL_FU_B { 4 } else { 2 }..)?;
                let (start, end) = (header & 0x80 != 0, header & 0x40 != 0);

                if start {
                    fragment.is_none().then_some(())?;
                    let mut nal = vec![(indicator & 0xe0) | (header & 0x1f)];
                    nal.extend_from_slice(body);
                    fragment = Some(nal);
                } else {
                    fragment.as_mut()?.extend_from_slice(body);
                }

                if end {
                    nals.push(fragment.take()?);
                }
            }
            _ => return None,
        }
    }

    fragment.is_none().then_some(nals)
}

/// An access unit is a keyframe if it is an IDR or its first slice's `slice_type` is I (2 or 7).
pub fn is_keyframe(nals: &[Vec<u8>]) -> bool {
    let Some(slice) = nals
        .iter()
        .find(|nal| matches!(nal.first().map(|header| header & 0x1f), Some(NAL_SLICE | NAL_IDR)))
    else {
        return false;
    };

    if slice[0] & 0x1f == NAL_IDR {
        return true;
    }

    // first_mb_in_slice and slice_type in the slice header are both Exp-Golomb coded.
    let mut bits = Bits::new(&slice[1..]);
    bits.ue()
        .and_then(|_| bits.ue())
        .is_some_and(|slice_type| slice_type % 5 == 2)
}

/// The picture size an SPS describes, after its frame cropping.
pub fn picture_size(sps: &[u8]) -> Option<(u32, u32)> {
    let mut bits = Bits::new(sps.get(1..)?);
    let profile = bits.read(8)?;

    // constraint flags and level_idc, then seq_parameter_set_id
    bits.skip(16)?;
    bits.ue()?;

    // High profiles carry the chroma format, bit depths, and optional scaling lists.
    let mut chroma_format = 1;

    if matches!(
        profile,
        44 | 83 | 86 | 100 | 110 | 118 | 122 | 128 | 134 | 135 | 138 | 139 | 144 | 244
    ) {
        chroma_format = bits.ue()?;

        if chroma_format == 3 {
            bits.skip(1)?;
        }

        bits.ue()?;
        bits.ue()?;
        bits.skip(1)?;

        if bits.read(1)? == 1 {
            let lists = if chroma_format == 3 { 12 } else { 8 };

            for list in 0..lists {
                if bits.read(1)? == 1 {
                    skip_scaling_list(&mut bits, if list < 6 { 16 } else { 64 })?;
                }
            }
        }
    }

    bits.ue()?;

    match bits.ue()? {
        0 => {
            bits.ue()?;
        }
        1 => {
            bits.skip(1)?;
            bits.se()?;
            bits.se()?;

            for _ in 0..bits.ue()? {
                bits.se()?;
            }
        }
        _ => {}
    }

    // max_num_ref_frames and gaps_in_frame_num_value_allowed_flag
    bits.ue()?;
    bits.skip(1)?;

    let width = (bits.ue()? + 1) * 16;
    let map_units = bits.ue()? + 1;
    let frames_only = bits.read(1)?;
    let height = (2 - frames_only) * map_units * 16;

    if frames_only == 0 {
        bits.skip(1)?;
    }

    bits.skip(1)?;

    if bits.read(1)? == 0 {
        return Some((width, height));
    }

    // Cropping counts in chroma samples, and in field pairs when the picture can be interlaced.
    let (across, down) = match chroma_format {
        0 | 3 => (1, 1),
        2 => (2, 1),
        _ => (2, 2),
    };
    let down = down * (2 - frames_only);
    let (left, right, top, bottom) = (bits.ue()?, bits.ue()?, bits.ue()?, bits.ue()?);

    Some((
        width.checked_sub(across * (left + right))?,
        height.checked_sub(down * (top + bottom))?,
    ))
}

// Skips one scaling list, whose entries are deltas until one makes the next value 0.
fn skip_scaling_list(bits: &mut Bits, size: usize) -> Option<()> {
    let mut next = 8;

    for _ in 0..size {
        if next != 0 {
            next = (next + bits.se()? + 256) % 256;
        }

        if next == 0 {
            break;
        }
    }

    Some(())
}

// Reads `count` NAL units, each with a `u16` length prefix.
fn units(reader: &mut Reader, count: u8) -> Result<Vec<Vec<u8>>> {
    (0..count)
        .map(|_| {
            let len = reader.u16()? as usize;
            Ok(reader.bytes(len)?.to_vec())
        })
        .collect()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::{depacketize, is_keyframe, picture_size};
    use crate::media::unit::{AccessUnit, DecoderConfig, VideoCodec};

    const SPS: &[u8] = &[0x67, 0x64, 0x00, 0x34, 0xac];
    const PPS: &[u8] = &[0x68, 0xee, 0x3c, 0x80];

    // Config packet: the marker, an arbitrary header, then the `avc1` entry containing the `avcC` box.
    fn config_packet() -> Vec<u8> {
        let mut record = vec![1, 0x64, 0x00, 0x34, 0xff, 0xe1];
        record.extend_from_slice(&(SPS.len() as u16).to_be_bytes());
        record.extend_from_slice(SPS);
        record.push(1);
        record.extend_from_slice(&(PPS.len() as u16).to_be_bytes());
        record.extend_from_slice(PPS);

        let mut packet = vec![0x92, 0, 0, 0, 0, 0, 0x56, b'a', b'v', b'c', b'1', 9, 9, 9];
        packet.extend_from_slice(&((record.len() + 8) as u32).to_be_bytes());
        packet.extend_from_slice(b"avcC");
        packet.extend_from_slice(&record);
        packet.extend_from_slice(b"trailing box");
        packet
    }

    #[test]
    fn config_packet_yields_sps_and_pps_for_the_decoder() {
        let config = DecoderConfig::from_h264(&config_packet()).unwrap();
        assert_eq!((config.profile, config.level), (0x64, 0x34));
        assert_eq!(config.sps, [SPS]);
        assert_eq!(config.pps, [PPS]);
        assert_eq!(config.annex_b(), [&[0, 0, 0, 1], SPS, &[0, 0, 0, 1], PPS].concat());
    }

    #[test]
    fn truncated_config_packet_is_rejected_at_every_cut() {
        let packet = config_packet();
        let end = packet.len() - b"trailing box".len();

        for cut in 0..end {
            assert_eq!(DecoderConfig::from_h264(&packet[..cut]), None, "cut at {cut}");
        }
    }

    #[test]
    fn fu_a_fragments_rebuild_the_nal_header() {
        let nal = depacketize([&[0x7c, 0x85, 1, 2][..], &[0x7c, 0x05, 3][..], &[0x7c, 0x45, 4][..]]).unwrap();
        assert_eq!(nal, [vec![0x65, 1, 2, 3, 4]]);
    }

    #[test]
    fn missing_fragment_start_or_end_is_incomplete() {
        assert_eq!(depacketize([&[0x7c, 0x05, 3][..], &[0x7c, 0x45, 4][..]]), None);
        assert_eq!(depacketize([&[0x7c, 0x85, 1][..]]), None);
    }

    #[test]
    fn stap_a_splits_into_units_without_don() {
        let packet = [24, 0, 2, 0x67, 1, 0, 1, 0x68];
        assert_eq!(depacketize([&packet[..]]).unwrap(), [vec![0x67, 1], vec![0x68]]);
    }

    #[test]
    fn i_slice_inside_a_non_idr_nal_is_a_keyframe() {
        // first_mb_in_slice = 0 ('1'), slice_type = 7 ('0001000'), slice_type = 5 ('00110')
        assert!(is_keyframe(&[vec![0x41, 0b1000_1000, 0]]));
        assert!(!is_keyframe(&[vec![0x41, 0b1001_1000, 0]]));
        assert!(is_keyframe(&[vec![0x65, 0]]));
        assert!(!is_keyframe(&[SPS.to_vec()]));
    }

    #[test]
    fn emulation_prevention_bytes_are_skipped() {
        // first_mb_in_slice = 2^23 - 1 gives the RBSP 00 00 01 00 00 00 e0, which needs two emulation prevention bytes.
        // Only by skipping them does the reader get slice_type 2 ('011').
        let nal = vec![0x41, 0, 0, 3, 1, 0, 0, 3, 0, 0xe0];
        assert!(is_keyframe(&[nal]));
    }

    #[test]
    fn in_band_sps_is_detected() {
        let (with_sps, _) = AccessUnit::depacketize(VideoCodec::H264, 1, 0, 0, [SPS, &[0x65][..]]).unwrap();
        let (idr, donl) = AccessUnit::depacketize(VideoCodec::H264, 1, 0, 0, [&[0x65][..]]).unwrap();
        assert!(with_sps.has_sps());
        assert!(!idr.has_sps());
        assert!(idr.keyframe);
        assert_eq!(idr.annex_b(), [0, 0, 0, 1, 0x65]);
        assert_eq!(donl, None);
    }

    #[test]
    fn picture_size_applies_the_frame_cropping() {
        // x264's High profile SPS for 2800×1752, which crops 8 rows off 110 macroblocks, and Baseline for 1920×1080
        let high = [
            0x67, 0x64, 0x00, 0x32, 0xac, 0xd9, 0x40, 0x2b, 0xc0, 0xdd, 0xf9, 0x70, 0x11, 0x00, 0x00, 0x03, 0x00, 0x01,
            0x00, 0x00, 0x03, 0x00, 0x32, 0x0f, 0x18, 0x31, 0x96,
        ];
        let baseline = [
            0x67, 0x42, 0xc0, 0x28, 0xd9, 0x00, 0x78, 0x02, 0x27, 0xe5, 0xc0, 0x44, 0x00, 0x00, 0x03, 0x00, 0x04, 0x00,
            0x00, 0x03, 0x00, 0xc8, 0x3c, 0x60, 0xc9, 0x20,
        ];

        assert_eq!(picture_size(&high), Some((2800, 1752)));
        assert_eq!(picture_size(&baseline), Some((1920, 1080)));
        assert_eq!(picture_size(&high[..6]), None);
    }
}
