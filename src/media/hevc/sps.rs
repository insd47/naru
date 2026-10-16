//! The parts of an HEVC SPS this crate reads, and the short-term reference picture sets it carries.
use crate::media::bits::Bits;
use crate::media::vui;

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

// Skips an SPS's profile_tier_level: the general profile and level, the sub-layer presence flags padded to eight,
// then each sub-layer's profile (88 bits) and level (8 bits) when present.
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
