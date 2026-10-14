use crate::error::{Error, Result};

// The server replaces any larger pixel size with one of its own choosing. Portrait displays use the limit with width
// and height swapped.
const MAX_PIXELS: (u32, u32) = (3840, 2160);

const NAME_BYTES: usize = 120;
const DESCRIPTOR_BYTES: usize = 0x9c;
const MODE_BYTES: usize = 28;

// The dynamic resolution flag and the virtual display kind are required for a mid-session resize to be accepted.
const DYNAMIC: u32 = 1;
const VIRTUAL: u32 = 4;
const ROTATION: u32 = 7;

/// Virtual display configuration (SetDisplayConfiguration).
///
/// - Sending it hides the Mac's physical screen, and the server streams a virtual display of this size.
/// - Sending it again mid-session requests a resolution change. The size actually applied arrives as a `Layout`.
pub struct DisplayConfig {
    /// Name shown in the Mac's display list. Truncated past 119 bytes.
    pub name: String,
    /// Physical size in millimeters. The server uses it to compute DPI.
    pub physical_mm: (f32, f32),
    /// Mode table. The first mode is used as both the current and the preferred mode.
    pub modes: Vec<Mode>,
}

/// A single display mode.
#[derive(Clone, Copy)]
pub struct Mode {
    pub pixels: (u32, u32),
    pub logical: (u32, u32),
    pub refresh_hz: f64,
    pub hdr: bool,
}

impl DisplayConfig {
    /// Builds the message, failing when there are no modes or a pixel size exceeds the server's limit.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let Some(first) = self.modes.first() else {
            return Err(Error::Invalid("display config"));
        };

        let (max_width, max_height) = limit(first.pixels);
        let fits = |mode: &Mode| mode.pixels.0 <= max_width && mode.pixels.1 <= max_height;

        if !self.modes.iter().all(fits) || self.modes.len() > 64 {
            return Err(Error::Invalid("display config"));
        }

        let descriptor_len = DESCRIPTOR_BYTES + MODE_BYTES * self.modes.len();

        let mut message = vec![0x1d, 0];
        message.extend_from_slice(&((8 + descriptor_len) as u16).to_be_bytes());
        message.extend_from_slice(&1u16.to_be_bytes());
        message.extend_from_slice(&1u16.to_be_bytes());
        message.extend_from_slice(&0u32.to_be_bytes());

        let mut name = [0; NAME_BYTES];
        let bytes = self.name.as_bytes();
        let len = bytes.len().min(NAME_BYTES - 1);
        name[..len].copy_from_slice(&bytes[..len]);

        message.extend_from_slice(&(descriptor_len as u16).to_be_bytes());
        message.extend_from_slice(&name);
        message.extend_from_slice(&DYNAMIC.to_be_bytes());
        message.extend_from_slice(&VIRTUAL.to_be_bytes());
        message.extend_from_slice(&self.physical_mm.0.to_be_bytes());
        message.extend_from_slice(&self.physical_mm.1.to_be_bytes());
        message.extend_from_slice(&max_width.to_be_bytes());
        message.extend_from_slice(&max_height.to_be_bytes());
        message.extend_from_slice(&0u16.to_be_bytes());
        message.extend_from_slice(&0u16.to_be_bytes());
        message.extend_from_slice(&ROTATION.to_be_bytes());
        message.extend_from_slice(&(self.modes.len() as u16).to_be_bytes());

        for mode in &self.modes {
            message.extend_from_slice(&mode.pixels.0.to_be_bytes());
            message.extend_from_slice(&mode.pixels.1.to_be_bytes());
            message.extend_from_slice(&mode.logical.0.to_be_bytes());
            message.extend_from_slice(&mode.logical.1.to_be_bytes());
            message.extend_from_slice(&mode.refresh_hz.to_be_bytes());
            message.extend_from_slice(&u32::from(mode.hdr).to_be_bytes());
        }

        Ok(message)
    }
}

// This app's virtual display. The name and physical size only show up in the Mac's display settings.
impl From<Mode> for DisplayConfig {
    fn from(mode: Mode) -> Self {
        Self {
            name: "Naru".into(),
            physical_mm: (369.45, 207.82),
            modes: vec![mode],
        }
    }
}

impl Mode {
    /// Builds a 60 Hz SDR mode from a logical size and scale. A scale of 2 is Retina.
    pub fn new(logical: (u32, u32), scale: f64) -> Self {
        let scaled = |value: u32| (f64::from(value) * scale).round().clamp(1.0, f64::from(u32::MAX)) as u32;

        Self {
            pixels: (scaled(logical.0), scaled(logical.1)),
            logical,
            refresh_hz: 60.0,
            hdr: false,
        }
    }

    /// Builds a mode whose logical size is shrunk, keeping the aspect ratio, until its pixels fit the server's limit
    /// (3840×2160 landscape, 2160×3840 portrait).
    pub fn fitting(logical: (u32, u32), scale: f64) -> Self {
        let (max_width, max_height) = limit(logical);
        let ratio = (f64::from(max_width) / (f64::from(logical.0) * scale))
            .min(f64::from(max_height) / (f64::from(logical.1) * scale))
            .min(1.0);
        let shrink = |value: u32| ((f64::from(value) * ratio).floor() as u32).max(1);

        Self::new((shrink(logical.0), shrink(logical.1)), scale)
    }
}

// Pixel limit for this orientation
fn limit((width, height): (u32, u32)) -> (u32, u32) {
    let (long, short) = MAX_PIXELS;
    if height > width { (short, long) } else { (long, short) }
}

#[cfg(test)]
mod tests {
    use super::{DisplayConfig, Mode};

    fn config(modes: Vec<Mode>) -> DisplayConfig {
        DisplayConfig {
            name: "Naru".into(),
            physical_mm: (369.45, 207.82),
            modes,
        }
    }

    #[test]
    fn five_modes_make_a_308_byte_message() {
        let message = config(vec![Mode::new((1280, 800), 2.0); 5]).encode().unwrap();

        assert_eq!(message.len(), 308);
        assert_eq!(&message[..12], [0x1d, 0, 0x01, 0x30, 0, 1, 0, 1, 0, 0, 0, 0]);
        assert_eq!(&message[12..14], 296u16.to_be_bytes());
        assert_eq!(&message[12 + 0x7a..12 + 0x82], [0, 0, 0, 1, 0, 0, 0, 4]);
        assert_eq!(&message[12 + 0x96..12 + 0x9c], [0, 0, 0, 7, 0, 5]);

        let mode = &message[12 + 0x9c..12 + 0x9c + 28];
        assert_eq!(
            &mode[..16],
            [0, 0, 0x0a, 0, 0, 0, 0x06, 0x40, 0, 0, 0x05, 0, 0, 0, 0x03, 0x20]
        );
        assert_eq!(&mode[16..24], 60f64.to_be_bytes());
    }

    #[test]
    fn pixels_beyond_the_server_limit_are_refused() {
        assert!(config(vec![Mode::new((2000, 1200), 2.0)]).encode().is_err());
        assert!(config(Vec::new()).encode().is_err());
    }

    // A portrait iPad (834×1210@2) failed to connect because it exceeded the landscape height limit.
    // The server accepts a portrait 1668×2420 as is.
    #[test]
    fn portrait_modes_use_the_rotated_limit() {
        let portrait = Mode::fitting((834, 1210), 2.0);
        assert_eq!(portrait.pixels, (1668, 2420));

        let message = config(vec![portrait]).encode().unwrap();
        assert_eq!(
            &message[12 + 0x8a..12 + 0x92],
            [2160u32.to_be_bytes(), 3840u32.to_be_bytes()].concat()
        );

        assert_eq!(Mode::fitting((2000, 1200), 2.0).pixels, (3600, 2160));
        assert_eq!(Mode::fitting((1200, 2400), 2.0).pixels, (1920, 3840));
    }
}
