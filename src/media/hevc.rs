//! HEVC RTP 페이로드(RFC 7798, DONL 없음)입니다.
//!
//! - 띠 1개로 요청하면 서버는 DONL 없는 표준 형식으로 보냅니다. 띠 4개일 때만 모든 구조에 DONL 2바이트가 붙습니다(2026-10-12 실측).
//! - 파라미터 세트(VPS, SPS, PPS)는 설정 패킷 없이 키프레임과 함께 스트림 안으로 옵니다.
use super::unit::{DecoderConfig, VideoCodec};
use crate::reader::Reader;

const NAL_VPS: u8 = 32;
/// SPS NAL 유형
pub const NAL_SPS: u8 = 33;
const NAL_PPS: u8 = 34;
const NAL_AP: u8 = 48;
const NAL_FU: u8 = 49;

// IRAP(BLA, IDR, CRA) 그림의 NAL 유형 범위입니다.
const IRAP: std::ops::RangeInclusive<u8> = 16..=21;

impl DecoderConfig {
    /// 접근 단위에 든 파라미터 세트로 디코더 설정을 만듭니다. VPS, SPS, PPS 가운데 하나라도 없으면 `None`입니다.
    pub fn from_hevc(nals: &[Vec<u8>]) -> Option<Self> {
        let of = |kind| -> Vec<Vec<u8>> { nals.iter().filter(|nal| nal_type(nal) == Some(kind)).cloned().collect() };
        let (vps, sps, pps) = (of(NAL_VPS), of(NAL_SPS), of(NAL_PPS));

        if vps.is_empty() || pps.is_empty() {
            return None;
        }

        // SPS의 profile_tier_level에서 general_profile_idc는 둘째 바이트의 하위 5비트, general_level_idc는 열세째 바이트입니다.
        let rbsp = rbsp(sps.first()?.get(2..)?, 13);
        let profile = *rbsp.get(1)? & 0x1f;
        let level = *rbsp.get(12)?;

        Some(Self {
            codec: VideoCodec::Hevc,
            profile,
            compatibility: 0,
            level,
            vps,
            sps,
            pps,
        })
    }
}

/// NAL 머리의 유형. 머리가 2바이트보다 짧으면 `None`입니다.
pub fn nal_type(nal: &[u8]) -> Option<u8> {
    (nal.len() >= 2).then(|| (nal[0] >> 1) & 0x3f)
}

/// 한 접근 단위의 페이로드들을 NAL 목록으로 되돌립니다. 조각이 빠졌거나 형식이 맞지 않으면 `None`입니다.
pub fn depacketize<'a>(payloads: impl IntoIterator<Item = &'a [u8]>) -> Option<Vec<Vec<u8>>> {
    let mut nals = Vec::new();
    let mut fragment: Option<Vec<u8>> = None;

    for payload in payloads {
        match nal_type(payload)? {
            0..NAL_AP => {
                fragment.is_none().then_some(())?;
                nals.push(payload.to_vec());
            }
            NAL_AP => {
                fragment.is_none().then_some(())?;
                let mut reader = Reader::new(&payload[2..], "AP");

                while reader.remaining() > 0 {
                    let len = reader.u16().ok()? as usize;
                    nals.push(reader.bytes(len).ok()?.to_vec());
                }
            }
            NAL_FU => {
                let header = *payload.get(2)?;
                let body = &payload[3..];
                let (start, end) = (header & 0x80 != 0, header & 0x40 != 0);

                // 원래 NAL 머리는 FU 머리의 유형으로 첫 바이트의 유형 자리만 바꾼 것입니다.
                if start {
                    fragment.is_none().then_some(())?;
                    let mut nal = vec![(payload[0] & 0x81) | ((header & 0x3f) << 1), payload[1]];
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

/// IRAP 그림이 들어 있으면 키프레임입니다.
pub fn is_keyframe(nals: &[Vec<u8>]) -> bool {
    nals.iter()
        .any(|nal| nal_type(nal).is_some_and(|kind| IRAP.contains(&kind)))
}

// 앞에서부터 `len`바이트까지 에뮬레이션 방지 바이트(00 00 03의 03)를 뗀 RBSP입니다.
fn rbsp(bytes: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut zeros = 0;

    for &byte in bytes {
        if out.len() == len {
            break;
        }

        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }

        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{depacketize, is_keyframe};
    use crate::media::unit::{AccessUnit, DecoderConfig, VideoCodec};

    // macOS 27 Mac mini가 2420×1668 가상 디스플레이로 보낸 파라미터 세트입니다(2026-10-12 캡처).
    const VPS: &[u8] = &[
        0x40, 0x01, 0x0c, 0x01, 0xff, 0xff, 0x04, 0x08, 0x00, 0x00, 0x03, 0x00, 0xbe, 0x08, 0x00, 0x00, 0x03, 0x00,
        0x00, 0x96, 0x0c, 0xc0, 0x90,
    ];
    const SPS: &[u8] = &[
        0x42, 0x01, 0x01, 0x04, 0x08, 0x00, 0x00, 0x03, 0x00, 0xbe, 0x08, 0x00, 0x00, 0x03, 0x00, 0x00, 0x96, 0x90,
        0x00, 0x26, 0x04, 0x00, 0xd2, 0x38, 0xd8, 0xdc, 0x40, 0x67, 0x72, 0x29, 0x0c, 0x5c, 0xfe, 0x27,
    ];
    const PPS: &[u8] = &[0x44, 0x01, 0xc0, 0x72, 0xf0, 0x5b, 0x24];
    const IDR: &[u8] = &[0x28, 0x01, 0xaf, 0x01];

    fn aggregate(nals: &[&[u8]]) -> Vec<u8> {
        let mut packet = vec![0x60, 0x01];

        for nal in nals {
            packet.extend_from_slice(&(nal.len() as u16).to_be_bytes());
            packet.extend_from_slice(nal);
        }

        packet
    }

    #[test]
    fn aggregation_packet_splits_without_don() {
        let nals = depacketize([aggregate(&[VPS, SPS, PPS]).as_slice()]).unwrap();
        assert_eq!(nals, [VPS, SPS, PPS]);
    }

    #[test]
    fn aggregation_size_past_the_payload_is_rejected() {
        let mut packet = aggregate(&[PPS]);
        packet[3] += 1;
        assert_eq!(depacketize([packet.as_slice()]), None);
    }

    #[test]
    fn fragments_rebuild_the_nal_header() {
        // IDR_N_LP(20)을 셋으로 나눈 FU입니다. 첫 바이트의 유형 자리에는 49가 들어 있습니다.
        let nals = depacketize([
            &[0x62, 0x01, 0x94, 1, 2][..],
            &[0x62, 0x01, 0x14, 3][..],
            &[0x62, 0x01, 0x54, 4][..],
        ]);
        assert_eq!(nals.unwrap(), [vec![0x28, 0x01, 1, 2, 3, 4]]);
    }

    #[test]
    fn missing_fragment_start_or_end_is_incomplete() {
        assert_eq!(
            depacketize([&[0x62, 0x01, 0x14, 3][..], &[0x62, 0x01, 0x54, 4][..]]),
            None
        );
        assert_eq!(depacketize([&[0x62, 0x01, 0x94, 1][..]]), None);
        assert_eq!(depacketize([&[0x62, 0x01][..]]), None);
    }

    #[test]
    fn unknown_payload_structures_are_rejected() {
        assert_eq!(depacketize([&[0x64, 0x01, 0][..]]), None);
        assert_eq!(depacketize([&[0x40][..]]), None);
    }

    #[test]
    fn irap_pictures_are_keyframes() {
        assert!(is_keyframe(&[IDR.to_vec()]));
        assert!(is_keyframe(&[vec![0x2a, 0x01]]));
        assert!(!is_keyframe(&[vec![0x02, 0x01]]));
        assert!(!is_keyframe(&[SPS.to_vec()]));
    }

    // 캡처한 SPS의 profile_tier_level에는 에뮬레이션 방지 바이트가 두 번 들어 있어, 떼지 않으면 레벨 자리가 어긋납니다.
    #[test]
    fn config_comes_from_in_band_parameter_sets() {
        let nals = [VPS.to_vec(), SPS.to_vec(), PPS.to_vec(), IDR.to_vec()];
        let config = DecoderConfig::from_hevc(&nals).unwrap();

        assert_eq!(config.codec, VideoCodec::Hevc);
        assert_eq!((config.profile, config.level), (4, 150));
        assert_eq!(
            (config.vps, config.sps, config.pps),
            (vec![VPS.to_vec()], vec![SPS.to_vec()], vec![PPS.to_vec()])
        );
        assert_eq!(DecoderConfig::from_hevc(&nals[1..]), None);
    }

    #[test]
    fn in_band_sps_is_detected() {
        let unit =
            AccessUnit::depacketize(VideoCodec::Hevc, 1, 0, [aggregate(&[VPS, SPS, PPS]).as_slice(), IDR]).unwrap();
        assert!(unit.keyframe);
        assert!(unit.has_sps());
        assert!(
            !AccessUnit::depacketize(VideoCodec::Hevc, 1, 0, [IDR])
                .unwrap()
                .has_sps()
        );
    }
}
