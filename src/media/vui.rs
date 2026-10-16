//! The start of the video usability information (VUI), which H.264 and HEVC share.
use super::bits::Bits;

// colour_primaries of SMPTE EG 432-1, Display P3
const DISPLAY_P3: u32 = 12;
const EXTENDED_SAR: u32 = 255;

/// Reads a VUI up to its colour description and returns whether the primaries are Display P3.
pub fn display_p3(bits: &mut Bits) -> Option<bool> {
    // aspect_ratio_info, with the sample aspect ratio spelled out after the extended marker
    if bits.read(1)? == 1 && bits.read(8)? == EXTENDED_SAR {
        bits.skip(32)?;
    }

    // overscan_info, then video_signal_type: video_format and video_full_range_flag before the colour description
    if bits.read(1)? == 1 {
        bits.skip(1)?;
    }

    if bits.read(1)? == 0 {
        return Some(false);
    }

    bits.skip(4)?;
    Some(bits.read(1)? == 1 && bits.read(8)? == DISPLAY_P3)
}
