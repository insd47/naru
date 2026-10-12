//! 화면에 닿은 손가락을 Mac 트랙패드의 동작으로 바꿉니다.
use crate::message::gesture::Phase;
use std::collections::HashMap;
use std::time::{Duration, Instant};

// 이 시간 안에 이만큼보다 덜 움직이고 떼면 탭입니다. 좌표는 레이아웃 픽셀입니다.
const TAP: Duration = Duration::from_millis(250);
const TAP_SLOP: f32 = 20.0;

// 탭한 뒤 이 시간 안에 다시 누르면 버튼을 누른 채 끌기를 시작합니다. 바로 떼면 두 번째 클릭이 되어 Mac이 더블 클릭으로 받습니다.
const DRAG_AFTER_TAP: Duration = Duration::from_millis(300);

// 두 손가락이 처음으로 이만큼 움직이면 확대인지 스크롤인지 정하고, 모두 뗄 때까지 바꾸지 않습니다.
const PINCH_SLOP: f32 = 20.0;
const SCROLL_SLOP: f32 = 10.0;

/// 손가락 하나의 단계입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TouchPhase {
    Began,
    Moved,
    Ended,
    Cancelled,
}

/// 손가락 하나의 터치. 위치는 레이아웃 픽셀이며, 화면 밖이어도 자르지 않습니다.
#[derive(Debug, Clone, Copy)]
pub struct Touch {
    pub id: u32,
    pub phase: TouchPhase,
    pub at: (f32, f32),
}

/// 터치에서 나온 트랙패드 동작입니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    /// 커서를 이만큼 옮깁니다(레이아웃 픽셀).
    Move((f32, f32)),
    /// 버튼 상태를 바꿉니다. 비트는 포인터 이벤트와 같습니다(왼쪽 1, 오른쪽 2).
    Buttons(u8),
    /// 트랙패드 스크롤. 이동량은 손가락이 움직인 레이아웃 픽셀입니다.
    Scroll { phase: Phase, delta: (f32, f32) },
    /// 확대. 배율은 시작할 때 1.0이고 누적됩니다.
    Magnify { phase: Phase, scale: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    /// 한 손가락. 커서를 옮기고, 떼면 탭인지 봅니다.
    Point,
    /// 탭 직후 다시 누른 한 손가락. 왼쪽 버튼을 누른 채 옮깁니다.
    Drag,
    /// 두 손가락이 아직 움직이지 않았습니다. 떼면 오른쪽 클릭입니다.
    Pair,
    Scroll,
    /// 시작할 때의 두 손가락 거리
    Pinch(f32),
    /// 세 손가락 이상. 시스템 제스처이므로 모두 뗄 때까지 아무것도 보내지 않습니다.
    Ignore,
}

/// 손가락의 터치를 트랙패드 동작으로 바꿉니다. 화면 전체가 하나의 트랙패드입니다.
///
/// - 한 손가락은 커서를 옮기고, 탭하면 왼쪽 클릭입니다. 탭한 직후 다시 누른 채 움직이면 끌기입니다.
/// - 두 손가락은 탭하면 오른쪽 클릭, 움직이면 스크롤이나 확대입니다. 손가락을 하나만 떼어도 모두 뗄 때까지 그 동작을 이어 갑니다.
/// - 세 손가락 이상은 iPadOS나 Android의 시스템 제스처이므로, 하던 동작을 끝내고 무시합니다.
pub struct Touchpad {
    fingers: HashMap<u32, (f32, f32)>,
    mode: Option<Mode>,
    // 첫 손가락이 닿은 시각과, 그 뒤로 움직인 거리
    began: Instant,
    travel: f32,
    // 두 손가락 제스처의 직전 중심과, 두 손가락이 닿았을 때의 거리
    origin: (f32, f32),
    base_spread: f32,
    last_tap: Option<Instant>,
}

impl Touchpad {
    pub fn new() -> Self {
        Self {
            fingers: HashMap::new(),
            mode: None,
            began: Instant::now(),
            travel: 0.0,
            origin: (0.0, 0.0),
            base_spread: 0.0,
            last_tap: None,
        }
    }

    pub fn handle(&mut self, touch: Touch, now: Instant) -> Vec<Gesture> {
        match touch.phase {
            TouchPhase::Began => self.begin(touch, now),
            TouchPhase::Moved => self.moved(touch),
            TouchPhase::Ended | TouchPhase::Cancelled => self.end(touch, now),
        }
    }

    fn begin(&mut self, touch: Touch, now: Instant) -> Vec<Gesture> {
        self.fingers.insert(touch.id, touch.at);

        match (self.fingers.len(), self.mode) {
            (1, _) => {
                self.began = now;
                self.travel = 0.0;

                let dragging = self
                    .last_tap
                    .take()
                    .is_some_and(|tap| now.duration_since(tap) <= DRAG_AFTER_TAP);
                self.mode = Some(if dragging { Mode::Drag } else { Mode::Point });
                if dragging {
                    vec![Gesture::Buttons(1)]
                } else {
                    Vec::new()
                }
            }
            (2, Some(Mode::Point)) => {
                self.mode = Some(Mode::Pair);
                self.origin = self.centroid();
                self.base_spread = self.spread();
                Vec::new()
            }
            (2, _) => Vec::new(),
            _ => {
                let finished = self.finish();
                self.mode = Some(Mode::Ignore);
                finished
            }
        }
    }

    fn moved(&mut self, touch: Touch) -> Vec<Gesture> {
        let Some(previous) = self.fingers.insert(touch.id, touch.at) else {
            return Vec::new();
        };

        match self.mode {
            Some(Mode::Point | Mode::Drag) if self.fingers.len() == 1 => {
                let delta = (touch.at.0 - previous.0, touch.at.1 - previous.1);
                self.travel += delta.0.hypot(delta.1);
                vec![Gesture::Move(delta)]
            }
            Some(Mode::Pair) if self.fingers.len() == 2 => self.classify(),
            Some(Mode::Scroll) => self.scroll(Phase::Changed),
            Some(Mode::Pinch(base)) if self.fingers.len() == 2 => vec![Gesture::Magnify {
                phase: Phase::Changed,
                scale: self.spread() / base,
            }],
            _ => Vec::new(),
        }
    }

    fn end(&mut self, touch: Touch, now: Instant) -> Vec<Gesture> {
        if self.fingers.remove(&touch.id).is_none() {
            return Vec::new();
        }

        // 손가락이 남아 있으면 동작을 이어 갑니다. 중심이 남은 손가락으로 건너뛰지 않도록 기준을 옮깁니다.
        if !self.fingers.is_empty() {
            self.origin = self.centroid();
            return Vec::new();
        }

        let tapped = now.duration_since(self.began) <= TAP && self.travel <= TAP_SLOP;
        let gestures = match self.mode {
            Some(Mode::Point) if tapped => {
                self.last_tap = Some(now);
                vec![Gesture::Buttons(1), Gesture::Buttons(0)]
            }
            Some(Mode::Pair) if tapped => vec![Gesture::Buttons(2), Gesture::Buttons(0)],
            _ => self.finish(),
        };

        self.mode = None;
        gestures
    }

    // 두 손가락이 처음 움직인 정도로 확대인지 스크롤인지 정합니다.
    fn classify(&mut self) -> Vec<Gesture> {
        let center = self.centroid();
        let moved = (center.0 - self.origin.0).hypot(center.1 - self.origin.1);
        self.travel = self.travel.max(moved);

        if self.base_spread > 0.0 && (self.spread() - self.base_spread).abs() > PINCH_SLOP {
            self.mode = Some(Mode::Pinch(self.base_spread));
            return vec![Gesture::Magnify {
                phase: Phase::Began,
                scale: 1.0,
            }];
        }

        if moved > SCROLL_SLOP {
            self.mode = Some(Mode::Scroll);
            let mut gestures = vec![Gesture::Scroll {
                phase: Phase::Began,
                delta: (0.0, 0.0),
            }];
            gestures.extend(self.scroll(Phase::Changed));
            return gestures;
        }

        Vec::new()
    }

    // 중심이 움직인 만큼 스크롤합니다. 손가락을 아래로 내리면 양수이고, iPad 트랙패드 스크롤과 같은 방향입니다.
    fn scroll(&mut self, phase: Phase) -> Vec<Gesture> {
        let center = self.centroid();
        let delta = (center.0 - self.origin.0, center.1 - self.origin.1);
        self.origin = center;
        vec![Gesture::Scroll { phase, delta }]
    }

    // 하던 동작을 끝냅니다. 누른 버튼은 떼고, 스크롤과 확대는 끝을 알립니다.
    fn finish(&mut self) -> Vec<Gesture> {
        match self.mode {
            Some(Mode::Drag) => vec![Gesture::Buttons(0)],
            Some(Mode::Scroll) => vec![Gesture::Scroll {
                phase: Phase::Ended,
                delta: (0.0, 0.0),
            }],
            Some(Mode::Pinch(base)) => vec![Gesture::Magnify {
                phase: Phase::Ended,
                scale: self.spread() / base,
            }],
            _ => Vec::new(),
        }
    }

    fn centroid(&self) -> (f32, f32) {
        let count = self.fingers.len().max(1) as f32;
        let (x, y) = self
            .fingers
            .values()
            .fold((0.0, 0.0), |(x, y), at| (x + at.0, y + at.1));
        (x / count, y / count)
    }

    // 두 손가락 사이의 거리. 두 손가락이 아니면 0입니다.
    fn spread(&self) -> f32 {
        let mut points = self.fingers.values();

        match (points.next(), points.next(), points.next()) {
            (Some(a), Some(b), None) => (a.0 - b.0).hypot(a.1 - b.1),
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Gesture, Touch, TouchPhase, Touchpad};
    use crate::message::gesture::Phase;
    use std::time::{Duration, Instant};

    struct Fingers {
        pad: Touchpad,
        now: Instant,
    }

    impl Fingers {
        fn new() -> Self {
            Self {
                pad: Touchpad::new(),
                now: Instant::now(),
            }
        }

        fn wait(&mut self, ms: u64) {
            self.now += Duration::from_millis(ms);
        }

        fn touch(&mut self, id: u32, phase: TouchPhase, at: (f32, f32)) -> Vec<Gesture> {
            self.pad.handle(Touch { id, phase, at }, self.now)
        }
    }

    use TouchPhase::{Began, Ended, Moved};

    #[test]
    fn one_finger_moves_the_cursor_by_its_travel() {
        let mut fingers = Fingers::new();
        assert_eq!(fingers.touch(1, Began, (100.0, 100.0)), []);
        assert_eq!(fingers.touch(1, Moved, (130.0, 90.0)), [Gesture::Move((30.0, -10.0))]);
        fingers.wait(400);
        assert_eq!(fingers.touch(1, Ended, (130.0, 90.0)), []);
    }

    #[test]
    fn short_still_touch_is_a_left_click_and_a_long_one_is_not() {
        let mut fingers = Fingers::new();
        fingers.touch(1, Began, (100.0, 100.0));
        fingers.wait(100);
        assert_eq!(
            fingers.touch(1, Ended, (102.0, 101.0)),
            [Gesture::Buttons(1), Gesture::Buttons(0)]
        );

        fingers.wait(1000);
        fingers.touch(1, Began, (100.0, 100.0));
        fingers.wait(400);
        assert_eq!(fingers.touch(1, Ended, (100.0, 100.0)), []);
    }

    #[test]
    fn pressing_again_right_after_a_tap_drags_with_the_left_button() {
        let mut fingers = Fingers::new();
        fingers.touch(1, Began, (100.0, 100.0));
        fingers.wait(80);
        fingers.touch(1, Ended, (100.0, 100.0));
        fingers.wait(150);

        assert_eq!(fingers.touch(2, Began, (100.0, 100.0)), [Gesture::Buttons(1)]);
        assert_eq!(fingers.touch(2, Moved, (160.0, 100.0)), [Gesture::Move((60.0, 0.0))]);
        assert_eq!(fingers.touch(2, Ended, (160.0, 100.0)), [Gesture::Buttons(0)]);
    }

    #[test]
    fn two_finger_tap_is_a_right_click() {
        let mut fingers = Fingers::new();
        fingers.touch(1, Began, (100.0, 100.0));
        fingers.touch(2, Began, (160.0, 100.0));
        fingers.wait(120);
        assert_eq!(fingers.touch(1, Ended, (100.0, 100.0)), []);
        assert_eq!(
            fingers.touch(2, Ended, (160.0, 100.0)),
            [Gesture::Buttons(2), Gesture::Buttons(0)]
        );
    }

    // 손가락 하나를 먼저 떼도 남은 손가락으로 커서를 옮기거나 중심이 건너뛰어 스크롤이 튀지 않습니다.
    #[test]
    fn two_fingers_scroll_until_both_lift() {
        let mut fingers = Fingers::new();
        fingers.touch(1, Began, (100.0, 100.0));
        fingers.touch(2, Began, (160.0, 100.0));

        assert_eq!(
            fingers.touch(1, Moved, (100.0, 140.0)),
            [
                Gesture::Scroll {
                    phase: Phase::Began,
                    delta: (0.0, 0.0)
                },
                Gesture::Scroll {
                    phase: Phase::Changed,
                    delta: (0.0, 20.0)
                },
            ]
        );
        assert_eq!(
            fingers.touch(2, Moved, (160.0, 140.0)),
            [Gesture::Scroll {
                phase: Phase::Changed,
                delta: (0.0, 20.0)
            }]
        );

        assert_eq!(fingers.touch(1, Ended, (100.0, 140.0)), []);
        assert_eq!(
            fingers.touch(2, Moved, (160.0, 150.0)),
            [Gesture::Scroll {
                phase: Phase::Changed,
                delta: (0.0, 10.0)
            }]
        );
        assert_eq!(
            fingers.touch(2, Ended, (160.0, 150.0)),
            [Gesture::Scroll {
                phase: Phase::Ended,
                delta: (0.0, 0.0)
            }]
        );
    }

    #[test]
    fn spreading_two_fingers_magnifies() {
        let mut fingers = Fingers::new();
        fingers.touch(1, Began, (100.0, 100.0));
        fingers.touch(2, Began, (200.0, 100.0));

        assert_eq!(
            fingers.touch(2, Moved, (250.0, 100.0)),
            [Gesture::Magnify {
                phase: Phase::Began,
                scale: 1.0
            }]
        );
        assert_eq!(
            fingers.touch(1, Moved, (50.0, 100.0)),
            [Gesture::Magnify {
                phase: Phase::Changed,
                scale: 2.0
            }]
        );
        fingers.touch(1, Ended, (50.0, 100.0));
        assert!(matches!(
            fingers.touch(2, Ended, (250.0, 100.0))[..],
            [Gesture::Magnify {
                phase: Phase::Ended,
                ..
            }]
        ));
    }

    #[test]
    fn a_third_finger_releases_the_drag_and_is_ignored() {
        let mut fingers = Fingers::new();
        fingers.touch(1, Began, (100.0, 100.0));
        fingers.wait(50);
        fingers.touch(1, Ended, (100.0, 100.0));
        fingers.wait(50);
        fingers.touch(1, Began, (100.0, 100.0));
        fingers.touch(2, Began, (150.0, 100.0));

        assert_eq!(fingers.touch(3, Began, (200.0, 100.0)), [Gesture::Buttons(0)]);
        assert_eq!(fingers.touch(1, Moved, (100.0, 200.0)), []);
        fingers.touch(1, Ended, (100.0, 200.0));
        fingers.touch(2, Ended, (150.0, 100.0));
        assert_eq!(fingers.touch(3, Ended, (200.0, 100.0)), []);
    }
}
