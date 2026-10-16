//! HEVC RTP payload (RFC 7798 with a decoding order number).
//!
//! - The video comes as four bands, and every structure carries a 2-byte DONL, each fragment of a split NAL included.
//!   With one band requested the server sends the standard format without DONL, which this module does not read.
//! - DONL counts pictures across all four bands in decoding order, and an LTR acknowledgment names a picture by it.
//! - Parameter sets (VPS, SPS, PPS) arrive in-stream with the keyframe, not in a config packet. They become the
//!   decoder config and are taken out of the access units.
use super::bits::Bits;
use super::unit::{DecoderConfig, VideoCodec};
use super::vui;
use crate::reader::Reader;

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

/// The parts of an SPS the decoder config and slice headers depend on
pub struct Sps {
    /// Picture size after the conformance window: one band's size for the banded video
    pub width: u32,
    pub height: u32,
    /// Whether the colors use Display P3 primaries (SMPTE EG 432-1), as a Mac's screen does
    pub display_p3: bool,
    pub separate_planes: bool,
    pub poc_bits: u32,
    pub sets: Vec<Rps>,
    pub long_term: bool,
}

impl Sps {
    pub fn parse(nal: &[u8]) -> Option<Self> {
        let mut bits = Bits::new(nal.get(2..)?);

        // sps_video_parameter_set_id, then the sub-layer count and temporal_id_nesting_flag
        bits.skip(4)?;
        let sub_layers = bits.read(3)?;
        bits.skip(1)?;
        skip_profile_tier_level(&mut bits, sub_layers)?;

        bits.ue()?;
        let chroma_format = bits.ue()?;
        let separate_planes = chroma_format == 3 && bits.read(1)? == 1;
        let (mut width, mut height) = (bits.ue()?, bits.ue()?);

        // The conformance window counts in chroma samples: 2 luma samples across for 4:2:0 and 4:2:2, 2 down for
        // 4:2:0.
        if bits.read(1)? == 1 {
            let (across, down) = match chroma_format {
                1 => (2, 2),
                2 => (2, 1),
                _ => (1, 1),
            };
            let (left, right, top, bottom) = (bits.ue()?, bits.ue()?, bits.ue()?, bits.ue()?);
            width = width.checked_sub(across * (left + right))?;
            height = height.checked_sub(down * (top + bottom))?;
        }

        // Bit depths
        bits.ue()?;
        bits.ue()?;
        let poc_bits = bits.ue()? + 4;

        let ordering = if bits.read(1)? == 1 { sub_layers + 1 } else { 1 };
        (0..ordering * 3).try_for_each(|_| bits.ue().map(drop))?;

        // Coding block and transform sizes and depths
        (0..6).try_for_each(|_| bits.ue().map(drop))?;

        if bits.read(1)? == 1 && bits.read(1)? == 1 {
            skip_scaling_lists(&mut bits)?;
        }

        // amp_enabled_flag and sample_adaptive_offset_enabled_flag, then PCM
        bits.skip(2)?;

        if bits.read(1)? == 1 {
            bits.skip(8)?;
            bits.ue()?;
            bits.ue()?;
            bits.skip(1)?;
        }

        let mut sets = Vec::new();

        for _ in 0..bits.ue()? {
            let set = Rps::parse(&mut bits, &sets, false)?;
            sets.push(set);
        }

        let long_term = bits.read(1)? == 1;

        // Long-term reference pictures, sps_temporal_mvp_enabled_flag and strong_intra_smoothing_enabled_flag, then
        // the VUI. Colors are a hint for showing the picture, so an SPS whose VUI cannot be read counts as sRGB.
        let mut colors = || {
            if long_term {
                for _ in 0..bits.ue()? {
                    bits.skip(poc_bits + 1)?;
                }
            }

            bits.skip(2)?;
            Some(bits.read(1)? == 1 && vui::display_p3(&mut bits)?)
        };
        let display_p3 = colors().unwrap_or(false);

        Some(Self {
            width,
            height,
            display_p3,
            separate_planes,
            poc_bits,
            sets,
            long_term,
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

/// Skips an SPS's profile_tier_level: the general profile and level, the sub-layer presence flags padded to eight,
/// then each sub-layer's profile (88 bits) and level (8 bits) when present.
pub fn skip_profile_tier_level(bits: &mut Bits, sub_layers: u32) -> Option<()> {
    bits.skip(96)?;
    let present: Vec<(u32, u32)> = (0..sub_layers)
        .map(|_| Some((bits.read(1)?, bits.read(1)?)))
        .collect::<Option<_>>()?;

    if sub_layers > 0 {
        bits.skip(2 * (8 - sub_layers))?;
    }

    present
        .into_iter()
        .try_for_each(|(profile, level)| bits.skip(profile * 88 + level * 8))
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

/// A short-term reference picture set: the pictures before and after this one that stay kept, nearest first, as
/// (picture order count difference, used by this picture).
#[derive(Clone, Default)]
pub struct Rps {
    before: Vec<(i32, bool)>,
    after: Vec<(i32, bool)>,
}

impl Rps {
    // Reads a set, which may be predicted from an earlier one in `sets`. A slice's own set may name which one.
    pub fn parse(bits: &mut Bits, sets: &[Rps], in_slice: bool) -> Option<Self> {
        if sets.is_empty() || bits.read(1)? == 0 {
            let (before, after) = (bits.ue()?, bits.ue()?);
            return Some(Self {
                before: Self::explicit(bits, before, -1)?,
                after: Self::explicit(bits, after, 1)?,
            });
        }

        let back = if in_slice { bits.ue()? as usize + 1 } else { 1 };
        let source = sets.get(sets.len().checked_sub(back)?)?;
        let sign = if bits.read(1)? == 1 { -1 } else { 1 };
        let delta = sign * (i32::try_from(bits.ue()?).ok()? + 1);

        // For each picture of the source set, then the source picture itself: whether it is used, and whether it is
        // kept
        let count = source.before.len() + source.after.len() + 1;
        let flags: Vec<(bool, bool)> = (0..count)
            .map(|_| {
                let used = bits.read(1)? == 1;
                let kept = used || bits.read(1)? == 1;
                Some((used, kept))
            })
            .collect::<Option<_>>()?;

        Some(Self::predicted(source, delta, &flags))
    }

    // Each difference is coded as one less than its distance from the previous picture of the list.
    fn explicit(bits: &mut Bits, count: u32, direction: i32) -> Option<Vec<(i32, bool)>> {
        let mut position = 0;

        (0..count)
            .map(|_| {
                position += direction * (i32::try_from(bits.ue()?).ok()? + 1);
                Some((position, bits.read(1)? == 1))
            })
            .collect()
    }

    // Shifts the source set by `delta`, adding the source picture itself, as H.265 7.4.8 orders them.
    fn predicted(source: &Rps, delta: i32, flags: &[(bool, bool)]) -> Self {
        let before_count = source.before.len();
        let own = flags[flags.len() - 1];
        let shifted_before = source
            .before
            .iter()
            .enumerate()
            .map(|(index, (poc, _))| (poc + delta, flags[index]));
        let shifted_after = source
            .after
            .iter()
            .enumerate()
            .map(|(index, (poc, _))| (poc + delta, flags[before_count + index]));

        let mut before: Vec<(i32, bool)> = Vec::new();
        let mut after: Vec<(i32, bool)> = Vec::new();

        for (poc, (used, kept)) in shifted_after.clone().rev() {
            if poc < 0 && kept {
                before.push((poc, used));
            }
        }

        if delta < 0 && own.1 {
            before.push((delta, own.0));
        }

        for (poc, (used, kept)) in shifted_before.clone() {
            if poc < 0 && kept {
                before.push((poc, used));
            }
        }

        for (poc, (used, kept)) in shifted_before.rev() {
            if poc > 0 && kept {
                after.push((poc, used));
            }
        }

        if delta > 0 && own.1 {
            after.push((delta, own.0));
        }

        for (poc, (used, kept)) in shifted_after {
            if poc > 0 && kept {
                after.push((poc, used));
            }
        }

        Self { before, after }
    }

    pub fn deltas(&self) -> Vec<(i32, bool)> {
        self.before.iter().chain(&self.after).copied().collect()
    }
}

// scaling_list_data: for each size and matrix, either a reference to another matrix or its coefficients.
fn skip_scaling_lists(bits: &mut Bits) -> Option<()> {
    for size in 0..4u32 {
        let step = if size == 3 { 3 } else { 1 };

        for _ in (0..6).step_by(step) {
            if bits.read(1)? == 0 {
                bits.ue()?;
                continue;
            }

            if size > 1 {
                bits.se()?;
            }

            for _ in 0..64.min(1 << (4 + (size << 1))) {
                bits.se()?;
            }
        }
    }

    Some(())
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
