//! H.264 RTP 페이로드(RFC 6184, 비인터리브 모드)와 디코더 설정입니다.
//!
//! - 서버는 SPS와 PPS를 별도 NAL로 보내지 않고, 첫 바이트가 `0x92`인 설정 패킷에 ISO BMFF `avcC` 상자로 담아 보냅니다. 이 바이트는 금지 비트가 켜져 있어 유효한 NAL 머리가 아니므로, NAL 유형보다 먼저 확인합니다.
//! - 키프레임이 IDR(유형 5)이 아니라 유형 1 NAL 안의 I 슬라이스로 올 수 있습니다.
use super::unit::{DecoderConfig, VideoCodec};
use crate::error::Result;
use crate::reader::Reader;

/// 디코더 설정 패킷의 첫 바이트
pub const CONFIG_MARKER: u8 = 0x92;

const NAL_SLICE: u8 = 1;
const NAL_IDR: u8 = 5;
/// SPS NAL 유형
pub const NAL_SPS: u8 = 7;
const NAL_STAP_A: u8 = 24;
const NAL_FU_A: u8 = 28;
const NAL_FU_B: u8 = 29;

impl DecoderConfig {
    /// H.264 설정 패킷의 페이로드에서 `avcC` 상자를 찾아 디코더 설정을 만듭니다. 설정 패킷이 아니거나 형식이 맞지 않으면 `None`입니다.
    pub fn from_h264(payload: &[u8]) -> Option<Self> {
        if payload.first() != Some(&CONFIG_MARKER) {
            return None;
        }

        // `avc1` 샘플 항목 안의 `avcC` 상자를 찾습니다. 상자 크기는 유형 앞의 4바이트입니다.
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
            profile,
            compatibility,
            level,
            vps: Vec::new(),
            sps,
            pps,
        })
    }
}

/// 한 접근 단위의 페이로드들을 NAL 목록으로 되돌립니다. 조각이 빠졌거나 형식이 맞지 않으면 `None`입니다.
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

/// IDR이거나, 첫 슬라이스의 `slice_type`이 I(2 또는 7)이면 키프레임입니다.
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

    // 슬라이스 머리의 first_mb_in_slice, slice_type은 모두 지수 골롬 부호입니다.
    let mut bits = Bits::new(&slice[1..]);
    bits.ue()
        .and_then(|_| bits.ue())
        .is_some_and(|slice_type| slice_type % 5 == 2)
}

// `u16` 길이 접두가 붙은 NAL `count`개를 읽습니다.
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

// RBSP 비트를 앞에서부터 읽습니다. 에뮬레이션 방지 바이트(00 00 03의 03)는 건너뜁니다.
struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
    bit: u8,
    zeros: u8,
}

impl<'a> Bits<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            at: 0,
            bit: 0,
            zeros: 0,
        }
    }

    fn bit(&mut self) -> Option<u32> {
        if self.bit == 0 {
            if self.zeros >= 2 && self.bytes.get(self.at) == Some(&3) {
                self.at += 1;
                self.zeros = 0;
            }

            let byte = *self.bytes.get(self.at)?;
            self.zeros = if byte == 0 { self.zeros + 1 } else { 0 };
        }

        let value = (self.bytes[self.at] >> (7 - self.bit)) & 1;
        self.bit += 1;

        if self.bit == 8 {
            self.bit = 0;
            self.at += 1;
        }

        Some(u32::from(value))
    }

    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;

        while self.bit()? == 0 {
            zeros += 1;

            if zeros > 31 {
                return None;
            }
        }

        let mut value = 1u64;

        for _ in 0..zeros {
            value = value << 1 | u64::from(self.bit()?);
        }

        u32::try_from(value - 1).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::{depacketize, is_keyframe};
    use crate::media::unit::{AccessUnit, DecoderConfig, VideoCodec};

    const SPS: &[u8] = &[0x67, 0x64, 0x00, 0x34, 0xac];
    const PPS: &[u8] = &[0x68, 0xee, 0x3c, 0x80];

    // 설정 패킷: 표식 뒤에 임의의 머리, `avc1` 항목, 그 안의 `avcC` 상자가 옵니다.
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
        // first_mb_in_slice = 2^23 − 1이라 RBSP 00 00 01 00 00 00 e0에 방지 바이트가 두 번 들어간 경우입니다.
        // 방지 바이트를 건너뛰어야 slice_type 2('011')를 읽습니다.
        let nal = vec![0x41, 0, 0, 3, 1, 0, 0, 3, 0, 0xe0];
        assert!(is_keyframe(&[nal]));
    }

    #[test]
    fn in_band_sps_is_detected() {
        let with_sps = AccessUnit::depacketize(VideoCodec::H264, 1, 0, [SPS, &[0x65][..]]).unwrap();
        let idr = AccessUnit::depacketize(VideoCodec::H264, 1, 0, [&[0x65][..]]).unwrap();
        assert!(with_sps.has_sps());
        assert!(!idr.has_sps());
        assert!(idr.keyframe);
        assert_eq!(idr.annex_b(), [0, 0, 0, 1, 0x65]);
    }
}
