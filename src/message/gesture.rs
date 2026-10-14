//! The native input message (`0x17`) that carries scrolls and gestures.
//!
//! - Unlike other messages, it is plaintext inside the record.
//! - The body mirrors macOS scroll and gesture event fields. Phase values match `CGScrollPhase`, `CGGesturePhase`,
//!   and `CGMomentumScrollPhase`, and scroll direction follows CGEvent (positive is up and left).

const TYPE: u8 = 0x17;

const SCROLL: u16 = 0x0b;
const SCROLL_VERSION: u16 = 1;
const SCROLL_BYTES: usize = 58;

// Scroll event kind, telling continuous trackpad-like scrolling from a mouse wheel.
const CONTINUOUS: u32 = 6;
const DISCRETE: u32 = 2;

const MAGNIFY: u16 = 0x03;
const MAGNIFY_VERSION: u16 = 2;
const MAGNIFY_BYTES: usize = 36;

/// A gesture phase. Scroll and magnify share the same values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Phase {
    /// Fingers touched the trackpad but have not moved yet. Followed by `Began` once they move, or `Cancelled` when
    /// they lift without moving.
    MayBegin = 128,
    Began = 1,
    Changed = 2,
    Ended = 4,
    Cancelled = 8,
}

/// Phase of the momentum scroll the client keeps sending after the fingers lift. Momentum always ends with `Ended`,
/// including when fingers touch down again during it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MomentumPhase {
    Began = 1,
    Changed = 2,
    Ended = 3,
}

/// A scroll event. A trackpad carries a phase or momentum phase; a mouse wheel carries neither.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scroll {
    /// Horizontal and vertical distance in points. This is what apps actually scroll by.
    pub points: (i32, i32),
    /// The same distance in lines, used by apps that scroll line by line
    pub lines: (f32, f32),
    pub phase: Option<Phase>,
    pub momentum: Option<MomentumPhase>,
    /// True for continuous trackpad-like scrolling, false for a mouse wheel
    pub continuous: bool,
    /// Pointer position in pixels
    pub at: (u16, u16),
}

impl Scroll {
    pub fn encode(&self) -> [u8; SCROLL_BYTES] {
        let mut message = [0; SCROLL_BYTES];
        message[..8].copy_from_slice(&header(SCROLL_BYTES, SCROLL_VERSION, SCROLL));

        message[8..10].copy_from_slice(&whole_lines(self.lines.0).to_be_bytes());
        message[10..12].copy_from_slice(&whole_lines(self.lines.1).to_be_bytes());
        message[14..18].copy_from_slice(&fixed(self.lines.0));
        message[18..22].copy_from_slice(&fixed(self.lines.1));
        message[26..30].copy_from_slice(&self.points.0.to_be_bytes());
        message[30..34].copy_from_slice(&self.points.1.to_be_bytes());

        message[38..42].copy_from_slice(&self.phase.map_or(0, |phase| phase as u32).to_be_bytes());
        message[42..46].copy_from_slice(&self.momentum.map_or(0, |momentum| momentum as u32).to_be_bytes());

        let kind = if self.continuous { CONTINUOUS } else { DISCRETE };
        message[50..54].copy_from_slice(&kind.to_be_bytes());
        message[54..56].copy_from_slice(&self.at.0.to_be_bytes());
        message[56..58].copy_from_slice(&self.at.1.to_be_bytes());
        message
    }
}

/// A magnify gesture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Magnify {
    /// Magnification since the previous event. Positive zooms in, negative zooms out, and the scale changes by a factor
    /// of `1 + magnification`.
    pub magnification: f64,
    pub phase: Phase,
    /// Center of the magnification in pixels
    pub at: (u16, u16),
}

impl Magnify {
    pub fn encode(&self) -> [u8; MAGNIFY_BYTES] {
        let mut message = [0; MAGNIFY_BYTES];
        message[..8].copy_from_slice(&header(MAGNIFY_BYTES, MAGNIFY_VERSION, MAGNIFY));
        message[8..16].copy_from_slice(&self.magnification.to_be_bytes());
        message[16..18].copy_from_slice(&self.at.0.to_be_bytes());
        message[18..20].copy_from_slice(&self.at.1.to_be_bytes());
        message[24..28].copy_from_slice(&(self.phase as u32).to_be_bytes());
        message
    }
}

fn header(len: usize, version: u16, kind: u16) -> [u8; 8] {
    let [len_high, len_low] = ((len - 4) as u16).to_be_bytes();
    let [version_high, version_low] = version.to_be_bytes();
    let [kind_high, kind_low] = kind.to_be_bytes();
    [
        TYPE,
        0,
        len_high,
        len_low,
        version_high,
        version_low,
        kind_high,
        kind_low,
    ]
}

// Whole lines. Any movement counts as at least one line in its direction instead of truncating to zero, as macOS does.
fn whole_lines(lines: f32) -> i16 {
    let whole = lines.trunc();
    let whole = if whole == 0.0 && lines != 0.0 {
        lines.signum()
    } else {
        whole
    };
    whole.clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

// 16.16 fixed point. Out-of-range values saturate.
fn fixed(value: f32) -> [u8; 4] {
    let scaled = (f64::from(value) * 65536.0)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    (scaled as i32).to_be_bytes()
}

#[cfg(test)]
mod tests {
    use super::{Magnify, MomentumPhase, Phase, Scroll};

    fn scroll(points: (i32, i32), lines: (f32, f32)) -> Scroll {
        Scroll {
            points,
            lines,
            phase: Some(Phase::Changed),
            momentum: None,
            continuous: true,
            at: (1247, 578),
        }
    }

    // A trackpad scroll sent by Apple's client (vertical -14 points, -1.3 lines)
    #[test]
    fn trackpad_scroll_matches_the_apple_client_bytes() {
        let message = scroll((0, -14), (0.0, -1.3)).encode();
        assert_eq!(&message[..8], [0x17, 0, 0, 54, 0, 1, 0, 0x0b]);
        assert_eq!(&message[8..12], [0, 0, 0xff, 0xff]);
        assert_eq!(&message[18..22], [0xff, 0xfe, 0xb3, 0x33]);
        assert_eq!(&message[30..34], (-14i32).to_be_bytes());
        assert_eq!(&message[38..42], [0, 0, 0, 2]);
        assert_eq!(&message[50..], [0, 0, 0, 6, 0x04, 0xdf, 0x02, 0x42]);
    }

    #[test]
    fn horizontal_values_come_before_vertical_ones() {
        let message = scroll((17, 0), (1.7, 0.0)).encode();
        assert_eq!(&message[8..10], [0, 1]);
        assert_eq!(&message[14..18], [0, 1, 0xb3, 0x33]);
        assert_eq!(&message[26..30], 17i32.to_be_bytes());
        assert!(
            message[10..14]
                .iter()
                .chain(&message[18..26])
                .chain(&message[30..38])
                .all(|&byte| byte == 0)
        );
    }

    #[test]
    fn any_movement_counts_as_at_least_one_whole_line() {
        let message = scroll((0, 4), (0.0, 0.4)).encode();
        assert_eq!(&message[10..12], [0, 1]);

        let still = scroll((0, 0), (0.0, 0.0)).encode();
        assert_eq!(&still[10..12], [0, 0]);
    }

    #[test]
    fn wheel_is_discrete_without_phases() {
        let wheel = Scroll {
            phase: None,
            continuous: false,
            ..scroll((0, 9), (0.0, 1.0))
        };
        let message = wheel.encode();
        assert_eq!((message[41], message[45]), (0, 0));
        assert_eq!(&message[50..54], [0, 0, 0, 2]);
    }

    #[test]
    fn momentum_phase_sits_after_the_scroll_phase() {
        let momentum = Scroll {
            phase: None,
            momentum: Some(MomentumPhase::Ended),
            ..scroll((0, 0), (0.0, 0.0))
        };
        assert_eq!(&momentum.encode()[38..46], [0, 0, 0, 0, 0, 0, 0, 3]);
    }

    // A magnify sent by Apple's client (0.020996, changed)
    #[test]
    fn magnify_carries_a_double_increment() {
        let message = Magnify {
            magnification: f64::from_be_bytes([0x3f, 0x95, 0x80, 0, 0, 0, 0, 0]),
            phase: Phase::Changed,
            at: (755, 1040),
        }
        .encode();

        assert_eq!(&message[..8], [0x17, 0, 0, 32, 0, 2, 0, 0x03]);
        assert_eq!(&message[8..28], hex("3f9580000000000002f304100000000000000002"));
        assert!(message[28..].iter().all(|&byte| byte == 0));
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }
}
