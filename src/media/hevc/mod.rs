//! HEVC RTP payload (RFC 7798 with a decoding order number).
//!
//! - The video comes as four bands, and every structure carries a 2-byte DONL, each fragment of a split NAL included.
//!   With one band requested the server sends the standard format without DONL, which this module does not read.
//! - DONL counts pictures across all four bands in decoding order, and an LTR acknowledgment names a picture by it.
//! - Parameter sets (VPS, SPS, PPS) arrive in-stream with the keyframe, not in a config packet. They become the
//!   decoder config and are taken out of the access units.
use super::unit::{DecoderConfig, VideoCodec};
use crate::reader::Reader;
pub use sps::{Rps, Sps};

mod sps;

const NAL_VPS: u8 = 32;
pub const NAL_SPS: u8 = 33;
pub const NAL_PPS: u8 = 34;
const NAL_AP: u8 = 48;
const NAL_FU: u8 = 49;

// NAL type range of IRAP (BLA, IDR, CRA) pictures.
const IRAP: std::ops::RangeInclusive<u8> = 16..=21;

impl DecoderConfig {
    /// Builds a decoder config from the parameter sets in an access unit. Returns `None` if any of VPS, SPS, or PPS is
    /// missing.
    pub fn from_hevc(nals: &[Vec<u8>]) -> Option<Self> {
        let of = |kind| -> Vec<Vec<u8>> { nals.iter().filter(|nal| nal_type(nal) == Some(kind)).cloned().collect() };
        let (vps, sps, pps) = (of(NAL_VPS), of(NAL_SPS), of(NAL_PPS));

        if vps.is_empty() || pps.is_empty() {
            return None;
        }

        let parsed = Sps::parse(sps.first()?)?;

        Some(Self {
            codec: VideoCodec::Hevc,
            bands: VideoCodec::Hevc.bands(),
            width: parsed.width,
            height: parsed.height,
            display_p3: parsed.display_p3,
            vps,
            sps,
            pps,
        })
    }
}

/// Type from the NAL header, or `None` if the header is shorter than 2 bytes
pub fn nal_type(nal: &[u8]) -> Option<u8> {
    (nal.len() >= 2).then(|| (nal[0] >> 1) & 0x3f)
}

/// Whether a NAL unit is a VPS, SPS, or PPS
pub fn is_parameter_set(nal: &[u8]) -> bool {
    nal_type(nal).is_some_and(|kind| (NAL_VPS..=NAL_PPS).contains(&kind))
}

/// Turns one access unit's payloads back into NAL units and the unit's DONL. Returns `None` if a fragment is missing or
/// malformed.
pub fn depacketize<'a>(payloads: impl IntoIterator<Item = &'a [u8]>) -> Option<(Vec<Vec<u8>>, u16)> {
    let mut nals = Vec::new();
    let mut fragment: Option<Vec<u8>> = None;
    let mut donl = None;

    for payload in payloads {
        let kind = nal_type(payload)?;
        let at = if kind == NAL_FU { 3 } else { 2 };
        let mut reader = Reader::new(payload.get(at..)?, "HEVC payload");
        donl.get_or_insert(reader.u16().ok()?);
        let body = reader.rest();

        match kind {
            0..NAL_AP => {
                fragment.is_none().then_some(())?;
                let mut nal = payload[..2].to_vec();
                nal.extend_from_slice(body);
                nals.push(nal);
            }
            NAL_AP => {
                fragment.is_none().then_some(())?;
                let mut reader = Reader::new(body, "AP");

                while reader.remaining() > 0 {
                    let len = reader.u16().ok()? as usize;
                    nals.push(reader.bytes(len).ok()?.to_vec());
                }
            }
            NAL_FU => {
                let header = payload[2];
                let (start, end) = (header & 0x80 != 0, header & 0x40 != 0);

                // The original NAL header is the payload header with its type bits replaced by the type from the FU
                // header.
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

    fragment.is_none().then_some(())?;
    Some((nals, donl?))
}

/// An access unit is a keyframe if it contains an IRAP picture.
pub fn is_keyframe(nals: &[Vec<u8>]) -> bool {
    nals.iter()
        .any(|nal| nal_type(nal).is_some_and(|kind| IRAP.contains(&kind)))
}

#[cfg(test)]
mod tests {
    use super::{Sps, depacketize, is_keyframe};
    use crate::media::unit::{AccessUnit, DecoderConfig, VideoCodec};

    // Parameter sets captured from a Mac streaming a 2420×1668 virtual display in four bands
    const VPS: &[u8] = &[
        0x40, 0x01, 0x0c, 0x01, 0xff, 0xff, 0x04, 0x08, 0x00, 0x00, 0x03, 0x00, 0xbe, 0x08, 0x00, 0x00, 0x03, 0x00,
        0x00, 0x96, 0x04, 0x30, 0x24,
    ];
    const SPS: &[u8] = &[
        0x42, 0x01, 0x01, 0x04, 0x08, 0x00, 0x00, 0x03, 0x00, 0xbe, 0x08, 0x00, 0x00, 0x03, 0x00, 0x00, 0x96, 0x90,
        0x00, 0x26, 0x04, 0x03, 0x63, 0x8d, 0xf1, 0x00, 0x87, 0x72, 0x29, 0x04, 0x57, 0x3f, 0x89, 0xf8, 0x5f, 0xd0,
        0xbe, 0xb1, 0xbd, 0x70, 0xfe, 0xba, 0x08, 0xf5, 0xd4, 0x13, 0xab, 0xab, 0x0a, 0xd5, 0xd5, 0xc1, 0x7a, 0xba,
        0xba, 0x0c, 0xd5, 0xd5, 0xd4, 0x1b, 0xab, 0xab, 0xaa, 0x0e, 0xd5, 0xd5, 0xd5, 0x41, 0xfa, 0xba, 0xba, 0xaa,
        0x04, 0x35, 0x75, 0x75, 0x55, 0x29, 0xb8, 0x60, 0x68, 0x08, 0x04,
    ];
    const PPS: &[u8] = &[0x44, 0x01, 0xc0, 0x72, 0xf0, 0x5b, 0x24];
    const IDR: &[u8] = &[0x28, 0x01, 0xaf, 0x01];

    // Aggregation packet with DONL 7.
    fn aggregate(nals: &[&[u8]]) -> Vec<u8> {
        let mut packet = vec![0x60, 0x01, 0, 7];

        for nal in nals {
            packet.extend_from_slice(&(nal.len() as u16).to_be_bytes());
            packet.extend_from_slice(nal);
        }

        packet
    }

    #[test]
    fn aggregation_packet_splits_after_its_don() {
        let (nals, donl) = depacketize([aggregate(&[VPS, SPS, PPS]).as_slice()]).unwrap();
        assert_eq!(nals, [VPS, SPS, PPS]);
        assert_eq!(donl, 7);
    }

    #[test]
    fn aggregation_size_past_the_payload_is_rejected() {
        let mut packet = aggregate(&[PPS]);
        packet[5] += 1;
        assert_eq!(depacketize([packet.as_slice()]), None);
    }

    #[test]
    fn single_nal_drops_its_don() {
        let (nals, donl) = depacketize([&[0x02, 0x01, 0xff, 0xfe, 9, 8][..]]).unwrap();
        assert_eq!(nals, [vec![0x02, 0x01, 9, 8]]);
        assert_eq!(donl, 0xfffe);
    }

    // Every fragment repeats the DONL, so it comes off each one, including a fragment that carries nothing else.
    #[test]
    fn fragments_rebuild_the_nal_header_and_drop_every_don() {
        let (nals, donl) = depacketize([
            &[0x62, 0x01, 0x94, 0, 3, 1, 2][..],
            &[0x62, 0x01, 0x14, 0, 3][..],
            &[0x62, 0x01, 0x14, 0, 3, 3][..],
            &[0x62, 0x01, 0x54, 0, 3, 4][..],
        ])
        .unwrap();
        assert_eq!(nals, [vec![0x28, 0x01, 1, 2, 3, 4]]);
        assert_eq!(donl, 3);
    }

    #[test]
    fn missing_fragment_start_end_or_don_is_incomplete() {
        assert_eq!(
            depacketize([&[0x62, 0x01, 0x14, 0, 3, 3][..], &[0x62, 0x01, 0x54, 0, 3, 4][..]]),
            None
        );
        assert_eq!(depacketize([&[0x62, 0x01, 0x94, 0, 3, 1][..]]), None);
        assert_eq!(depacketize([&[0x62, 0x01, 0x94, 0][..]]), None);
        assert_eq!(depacketize([&[0x02, 0x01, 0][..]]), None);
        assert_eq!(depacketize([&[0x62, 0x01][..]]), None);
    }

    #[test]
    fn unknown_payload_structures_are_rejected() {
        assert_eq!(depacketize([&[0x64, 0x01, 0, 0, 0][..]]), None);
        assert_eq!(depacketize([&[0x40][..]]), None);
    }

    #[test]
    fn irap_pictures_are_keyframes() {
        assert!(is_keyframe(&[IDR.to_vec()]));
        assert!(is_keyframe(&[vec![0x2a, 0x01]]));
        assert!(!is_keyframe(&[vec![0x02, 0x01]]));
        assert!(!is_keyframe(&[SPS.to_vec()]));
    }

    #[test]
    fn config_comes_from_in_band_parameter_sets() {
        let nals = [VPS.to_vec(), SPS.to_vec(), PPS.to_vec(), IDR.to_vec()];
        let config = DecoderConfig::from_hevc(&nals).unwrap();

        assert_eq!(config.codec, VideoCodec::Hevc);
        assert_eq!((config.width, config.height, config.display_p3), (2420, 432, true));
        assert_eq!(
            (config.vps, config.sps, config.pps),
            (vec![VPS.to_vec()], vec![SPS.to_vec()], vec![PPS.to_vec()])
        );
        assert_eq!(DecoderConfig::from_hevc(&nals[1..]), None);
    }

    #[test]
    fn in_band_sps_is_detected() {
        let idr = [0x28, 0x01, 0, 8, 0xaf, 0x01];
        let (unit, donl) =
            AccessUnit::depacketize(VideoCodec::Hevc, 0, 0, [aggregate(&[VPS, SPS, PPS]).as_slice(), &idr]).unwrap();
        assert!(unit.keyframe);
        assert!(unit.has_sps());
        assert_eq!(donl, Some(7));
        assert!(
            !AccessUnit::depacketize(VideoCodec::Hevc, 0, 0, [&idr[..]])
                .unwrap()
                .0
                .has_sps()
        );
    }

    #[test]
    fn sps_reads_past_the_profile_and_level() {
        let parsed = Sps::parse(SPS).unwrap();
        assert_eq!((parsed.width, parsed.height), (2420, 432));
        assert!(Sps::parse(&SPS[..20]).is_none());
    }
}
