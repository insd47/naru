//! Which HEVC pictures decode intact, read from the reference picture set every slice carries.
//!
//! - A picture is intact when it arrived whole and every picture it predicts from is intact. A lost picture never
//!   becomes intact, so neither does anything predicted from it.
//! - The bands are one stream: they take turns in decoding order, with picture order counts a band apart, and a band's
//!   picture may predict from another band's.
//! - The Mac recovers from a loss with an ordinary P picture that predicts only from older pictures it kept, the ones
//!   the client acknowledged, so intact pictures resume without a keyframe.
//! - Long-term references are not read, since the Mac's SPS turns them off.
use super::bits::Bits;

const NAL_IDR_W_RADL: u8 = 19;
const NAL_IDR_N_LP: u8 = 20;
const NAL_SPS: u8 = 33;
const NAL_PPS: u8 = 34;

/// Follows a stream's pictures in decoding order and tells which decode intact.
#[derive(Default)]
pub struct References {
    sps: Option<Sps>,
    pps: Option<Pps>,
    // Picture order count of the last picture that later ones count from, and the intact pictures still kept
    previous: Option<i32>,
    intact: Vec<i32>,
}

impl References {
    /// Reads one access unit that arrived whole and returns whether it decodes intact, or `None` if its headers could
    /// not be read.
    pub fn intact(&mut self, nals: &[Vec<u8>]) -> Option<bool> {
        for nal in nals {
            match nal_type(nal) {
                Some(NAL_SPS) => self.sps = Sps::parse(nal),
                Some(NAL_PPS) => self.pps = Pps::parse(nal),
                _ => {}
            }
        }

        let slice = nals.iter().find(|nal| nal_type(nal).is_some_and(|kind| kind < 32))?;
        let (poc, kept) = self.slice(slice)?;
        let used: Vec<i32> = kept
            .iter()
            .filter(|(_, used)| *used)
            .map(|(delta, _)| poc + delta)
            .collect();
        let intact = used.iter().all(|reference| self.intact.contains(reference));

        // The reference set lists every picture still kept, so anything outside it is gone for good.
        self.intact
            .retain(|picture| kept.iter().any(|(delta, _)| poc + delta == *picture));

        if intact {
            self.intact.push(poc);
        }

        Some(intact)
    }

    // The picture order count of a picture's first slice and every picture its reference set keeps, as (delta, used now).
    fn slice(&mut self, nal: &[u8]) -> Option<(i32, Vec<(i32, bool)>)> {
        let (sps, pps) = (self.sps.as_ref()?, self.pps.as_ref()?);
        let kind = nal_type(nal)?;
        let temporal = nal.get(1)? & 7;
        let mut bits = Bits::new(nal.get(2..)?);

        // Only a picture's first slice is read; the others repeat its references.
        if bits.read(1)? == 0 {
            return None;
        }

        if (16..=23).contains(&kind) {
            bits.skip(1)?;
        }

        bits.ue()?;
        bits.skip(pps.extra_bits)?;
        bits.ue()?;
        bits.skip(u32::from(pps.output_flag) + 2 * u32::from(sps.separate_planes))?;

        if kind == NAL_IDR_W_RADL || kind == NAL_IDR_N_LP {
            self.previous = Some(0);
            self.intact.clear();
            return Some((0, Vec::new()));
        }

        let lsb = i32::try_from(bits.read(sps.poc_bits)?).ok()?;
        let set = if bits.read(1)? == 0 {
            Rps::parse(&mut bits, &sps.sets, true)?
        } else {
            let index = if sps.sets.len() > 1 {
                bits.read(ceil_log2(sps.sets.len()))?
            } else {
                0
            };
            sps.sets.get(index as usize)?.clone()
        };

        if sps.long_term {
            return None;
        }

        let poc = self.order(lsb, sps.poc_bits);

        // Sub-layer non-reference, RADL, and RASL pictures are not counted from.
        if temporal == 1 && !(kind <= 14 && kind % 2 == 0) && !(6..=9).contains(&kind) {
            self.previous = Some(poc);
        }

        Some((poc, set.deltas()))
    }

    // Derives the full picture order count from its low bits and the last picture counted from.
    fn order(&self, lsb: i32, bits: u32) -> i32 {
        let max = 1 << bits;
        let previous = self.previous.unwrap_or(0);
        let (previous_lsb, previous_msb) = (previous.rem_euclid(max), previous - previous.rem_euclid(max));

        let msb = if lsb < previous_lsb && previous_lsb - lsb >= max / 2 {
            previous_msb + max
        } else if lsb > previous_lsb && lsb - previous_lsb > max / 2 {
            previous_msb - max
        } else {
            previous_msb
        };

        msb + lsb
    }
}

// The parts of an SPS that slice headers depend on
struct Sps {
    separate_planes: bool,
    poc_bits: u32,
    sets: Vec<Rps>,
    long_term: bool,
}

impl Sps {
    fn parse(nal: &[u8]) -> Option<Self> {
        let mut bits = Bits::new(nal.get(2..)?);

        bits.skip(4)?;
        let sub_layers = bits.read(3)?;
        bits.skip(1)?;
        skip_profile_tier_level(&mut bits, sub_layers)?;

        bits.ue()?;
        let separate_planes = bits.ue()? == 3 && bits.read(1)? == 1;

        // Picture size, the conformance window, and bit depths
        bits.ue()?;
        bits.ue()?;

        if bits.read(1)? == 1 {
            (0..4).try_for_each(|_| bits.ue().map(drop))?;
        }

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

        Some(Self {
            separate_planes,
            poc_bits,
            sets,
            long_term: bits.read(1)? == 1,
        })
    }
}

// The parts of a PPS that slice headers depend on
struct Pps {
    output_flag: bool,
    extra_bits: u32,
}

impl Pps {
    fn parse(nal: &[u8]) -> Option<Self> {
        let mut bits = Bits::new(nal.get(2..)?);

        bits.ue()?;
        bits.ue()?;

        // dependent_slice_segments_enabled_flag would add a field to every slice but the first, which is all that is read.
        bits.skip(1)?;

        Some(Self {
            output_flag: bits.read(1)? == 1,
            extra_bits: bits.read(3)?,
        })
    }
}

// A short-term reference picture set: the pictures before and after this one that stay kept, nearest first, as
// (picture order count difference, used by this picture).
#[derive(Clone, Default)]
struct Rps {
    before: Vec<(i32, bool)>,
    after: Vec<(i32, bool)>,
}

impl Rps {
    // Reads a set, which may be predicted from an earlier one in `sets`. A slice's own set may name which one.
    fn parse(bits: &mut Bits, sets: &[Rps], in_slice: bool) -> Option<Self> {
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

        // For each picture of the source set, then the source picture itself: whether it is used, and whether it is kept
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

    fn deltas(&self) -> Vec<(i32, bool)> {
        self.before.iter().chain(&self.after).copied().collect()
    }
}

fn nal_type(nal: &[u8]) -> Option<u8> {
    (nal.len() >= 2).then(|| (nal[0] >> 1) & 0x3f)
}

// profile_tier_level: the general profile and level, the sub-layer presence flags padded to eight, then each sub-layer's
// profile (88 bits) and level (8 bits) when present.
fn skip_profile_tier_level(bits: &mut Bits, sub_layers: u32) -> Option<()> {
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

fn ceil_log2(value: usize) -> u32 {
    usize::BITS - (value - 1).leading_zeros()
}

#[cfg(test)]
mod tests {
    use super::{Pps, References, Sps};

    // Parameter sets and first slices a Mac sent during a 10% loss, from a capture of Apple's client
    const SPS: &str = "420101040800000300be08000003000099900037c40100e27c4021dc8a4115cfe27e17f42fac6f5c3fae823d7504eaeac2b575705eaeae8335757506eaeaea83b57575507eaeaeaa810d5d5d554a6e181a0201";
    const PPS: &str = "4401c072f05b24";
    const IDR: &str = "2801af04d610148b94a5d956569c45a0d04f96062a00c13590a4faff5c6e4a6e";
    // Band 1's picture lost in transit, band 0's recovery picture, and band 1's recovery picture, in decoding order
    const LOST: &str =
        "0201d215021eaaa9284003541b74404d61016710c19a54349c96d33569f03e3cd51afd55b4feb180fe04fbaace3defb0";
    const RECOVERY_BAND_0: &str =
        "0201d220076aaaa8d00dd06dd10135840b37b5cd735ac7318b5ae0f9025d236f47c0000a4feec2d3c71599a0972f80d8";
    const RECOVERY_BAND_1: &str =
        "0201d22107faaaaa3003741b74404d61016740bb19c4229c970b306cd06a42d2db075675beb280fe04fbaace3defb09d";

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }

    fn used(references: &mut References, slice: &str) -> (i32, Vec<i32>) {
        let (poc, kept) = references.slice(&hex(slice)).unwrap();
        (
            poc,
            kept.iter()
                .filter(|(_, used)| *used)
                .map(|(delta, _)| poc + delta)
                .collect(),
        )
    }

    #[test]
    fn parameter_sets_read_the_fields_slices_need() {
        let sps = Sps::parse(&hex(SPS)).unwrap();
        let pps = Pps::parse(&hex(PPS)).unwrap();

        assert_eq!(
            (sps.poc_bits, sps.sets.len(), sps.long_term, sps.separate_planes),
            (11, 16, false, false)
        );
        assert_eq!((pps.output_flag, pps.extra_bits), (false, 0));
    }

    // The recovery pictures predict from the picture just before them and from a far one the Mac kept, both of which
    // the client acknowledged, never from the lost picture.
    #[test]
    fn recovery_pictures_predict_around_the_lost_one() {
        let mut references = References {
            sps: Sps::parse(&hex(SPS)),
            pps: Pps::parse(&hex(PPS)),
            ..References::default()
        };

        assert_eq!(used(&mut references, LOST), (533, vec![532, 73]));
        assert_eq!(used(&mut references, RECOVERY_BAND_0), (544, vec![528, 73]));
        assert_eq!(used(&mut references, RECOVERY_BAND_1), (545, vec![544, 73]));
    }

    #[test]
    fn pictures_after_a_loss_are_intact_again_from_the_recovery_picture() {
        // 532 was lost, while 528 and the far 73 arrived intact.
        let mut references = References {
            sps: Sps::parse(&hex(SPS)),
            pps: Pps::parse(&hex(PPS)),
            intact: vec![73, 528],
            ..References::default()
        };

        assert_eq!(references.intact(&[hex(LOST)]), Some(false));
        assert_eq!(references.intact(&[hex(RECOVERY_BAND_0)]), Some(true));
        assert_eq!(references.intact(&[hex(RECOVERY_BAND_1)]), Some(true));
    }

    // The Mac answers some refresh requests with a keyframe, which starts over from nothing.
    #[test]
    fn a_keyframe_is_intact_and_clears_what_came_before() {
        let mut references = References {
            intact: vec![73, 528],
            ..References::default()
        };

        assert_eq!(references.intact(&[hex(SPS), hex(PPS), hex(IDR)]), Some(true));
        assert_eq!(references.intact, [0]);
    }
}
