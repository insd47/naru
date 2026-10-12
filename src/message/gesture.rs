//! 스크롤과 제스처를 싣는 네이티브 입력 메시지(`0x17`)입니다.
//!
//! - `17 00, u16 길이(전체 − 4), u16 버전, u16 하위 유형, 본문`. 다른 메시지와 달리 레코드 안에서 평문입니다.
//! - 본문은 macOS의 스크롤, 제스처 이벤트 필드를 그대로 옮긴 것입니다. 단계 값은 `CGScrollPhase`, `CGGesturePhase`,
//!   `CGMomentumScrollPhase`와 같고, 스크롤 방향도 CGEvent를 따릅니다(양수가 위, 왼쪽).

const TYPE: u8 = 0x17;

const SCROLL: u16 = 0x0b;
const SCROLL_VERSION: u16 = 1;
const SCROLL_BYTES: usize = 58;

// 스크롤 이벤트의 종류. 트랙패드처럼 연속적인 스크롤과 마우스 휠을 가릅니다.
const CONTINUOUS: u32 = 6;
const DISCRETE: u32 = 2;

const MAGNIFY: u16 = 0x03;
const MAGNIFY_VERSION: u16 = 2;
const MAGNIFY_BYTES: usize = 36;

/// 제스처의 단계입니다. 스크롤과 확대가 같은 값을 씁니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Phase {
    Began = 1,
    Changed = 2,
    Ended = 4,
    Cancelled = 8,
}

/// 손을 뗀 뒤 클라이언트가 이어 보내는 관성 스크롤의 단계입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Momentum {
    Began = 1,
    Changed = 2,
    Ended = 3,
}

/// 스크롤 이벤트입니다. 트랙패드는 단계나 관성 단계를 싣고, 마우스 휠은 둘 다 없습니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scroll {
    /// 가로, 세로 이동량(포인트). 앱이 실제로 스크롤하는 값입니다.
    pub points: (i32, i32),
    /// 같은 이동을 줄 단위로 나타낸 값. 줄 단위로 스크롤하는 앱이 씁니다.
    pub lines: (f32, f32),
    pub phase: Option<Phase>,
    pub momentum: Option<Momentum>,
    /// 트랙패드처럼 연속적인 스크롤이면 참, 마우스 휠이면 거짓
    pub continuous: bool,
    /// 포인터 위치(픽셀)
    pub at: (u16, u16),
}

impl Scroll {
    pub fn encode(&self) -> [u8; SCROLL_BYTES] {
        let mut message = [0; SCROLL_BYTES];
        message[..8].copy_from_slice(&header(SCROLL_BYTES, SCROLL_VERSION, SCROLL));

        // 가로가 앞, 세로가 뒤입니다. 정수 줄, 16.16 줄, 정수 포인트 순으로 놓입니다.
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

/// 확대 제스처입니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Magnify {
    /// 직전 이벤트 이후의 확대량. 양수가 확대, 음수가 축소이며 배율은 `1 + magnification`배씩 바뀝니다.
    pub magnification: f64,
    pub phase: Phase,
    /// 확대의 중심(픽셀)
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

// 정수 줄 수. 0으로 버리지 않고, 조금이라도 움직였으면 그 방향으로 한 줄입니다(macOS와 같음).
fn whole_lines(lines: f32) -> i16 {
    let whole = lines.trunc();
    let whole = if whole == 0.0 && lines != 0.0 {
        lines.signum()
    } else {
        whole
    };
    whole.clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

// 16.16 고정소수. 범위를 넘는 값은 끝에서 멈춥니다.
fn fixed(value: f32) -> [u8; 4] {
    let scaled = (f64::from(value) * 65536.0)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    (scaled as i32).to_be_bytes()
}

#[cfg(test)]
mod tests {
    use super::{Magnify, Momentum, Phase, Scroll};

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

    // Apple 클라이언트가 보낸 트랙패드 스크롤 하나(세로 -14포인트, -1.3줄)
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
            momentum: Some(Momentum::Ended),
            ..scroll((0, 0), (0.0, 0.0))
        };
        assert_eq!(&momentum.encode()[38..46], [0, 0, 0, 0, 0, 0, 0, 3]);
    }

    // Apple 클라이언트가 보낸 확대 하나(0.020996, 진행 중)
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
