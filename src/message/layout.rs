use crate::error::{Error, Result};
use crate::reader::Reader;

const MAX_DISPLAYS: u16 = 16;
const DISPLAY_BYTES: usize = 56;

/// The Mac's display layout. Multiple monitors each occupy a region of one combined canvas.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Canvas size in the coordinate space the Mac draws its UI in
    pub logical: (u16, u16),
    /// Size of the decoded video. Pointer coordinates use this space too.
    pub pixels: (u16, u16),
}

impl Layout {
    /// Parses the body of a layout rectangle, without its `u16` length.
    ///
    /// - Only the canvas size is used, so the per-display list is length-checked and skipped.
    pub fn parse(body: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(body, "display layout");
        reader.skip(2)?;

        let logical = (reader.u16()?, reader.u16()?);
        let pixels = (reader.u16()?, reader.u16()?);
        reader.skip(8)?;

        let count = reader.u16()?;

        if count > MAX_DISPLAYS {
            return Err(Error::Malformed("display layout"));
        }

        reader.skip(usize::from(count) * DISPLAY_BYTES)?;
        Ok(Self { logical, pixels })
    }
}

#[cfg(test)]
mod tests {
    use super::Layout;

    // Layout of a single Retina display, 1512×982 logical and 3024×1964 pixels.
    fn body() -> Vec<u8> {
        let mut body = Vec::new();

        for value in [5u16, 1512, 982, 3024, 1964] {
            body.extend_from_slice(&value.to_be_bytes());
        }

        body.extend_from_slice(&[0xff; 4]);
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&1u16.to_be_bytes());
        body.extend_from_slice(&2f64.to_be_bytes());
        body.extend_from_slice(&2f64.to_be_bytes());
        body.extend_from_slice(&1u32.to_be_bytes());

        for value in [0u16, 0, 982, 1512, 0, 0, 1964, 3024] {
            body.extend_from_slice(&value.to_be_bytes());
        }

        body.extend_from_slice(&[0; 20]);
        body
    }

    #[test]
    fn canvas_sizes_are_read_and_displays_skipped() {
        let layout = Layout::parse(&body()).unwrap();
        assert_eq!((layout.logical, layout.pixels), ((1512, 982), (3024, 1964)));
    }

    #[test]
    fn truncated_layout_is_an_error_at_every_cut() {
        let body = body();

        for cut in 0..body.len() {
            assert!(Layout::parse(&body[..cut]).is_err(), "cut at {cut}");
        }
    }
}
