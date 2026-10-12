use crate::message::client;
use crate::message::gesture::{Magnify, Momentum, Phase, Scroll};
use crate::record::RecordKeys;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use touch::{Gesture, Touchpad};
pub use touch::{Touch, TouchPhase};

mod touch;

// 줄 단위 값 하나가 몇 포인트인지입니다. Apple 클라이언트의 트랙패드 스크롤에서 약 10이고, 휠 한 칸도 이만큼 스크롤합니다.
const POINTS_PER_LINE: f32 = 10.0;

// 관성 스크롤입니다. 손을 떼기 직전의 이동으로 속도를 정하고, UIScrollView의 기본 감속(1ms마다 0.998배)으로 줄여 가며 보냅니다.
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);
const FRAME: Duration = Duration::from_millis(16);
const DECELERATION_PER_MS: f32 = 0.998;
const FLING_POINTS_PER_SEC: f32 = 100.0;
const STOP_POINTS_PER_SEC: f32 = 10.0;

/// 네이티브 계층이 보내는 입력입니다.
#[derive(Debug, Clone, Copy)]
pub enum Event {
    /// 포인터 위치(레이아웃 픽셀)와 버튼
    Pointer { buttons: u8, x: u16, y: u16 },
    /// 트랙패드 스크롤. 이동량은 포인트이고 방향은 CGEvent를 따릅니다(양수가 위, 왼쪽).
    Scroll { phase: Phase, delta: (f32, f32) },
    /// 마우스 휠. 칸 수이며 방향은 스크롤과 같습니다.
    Wheel { lines: (f32, f32) },
    /// 확대. 배율은 시작할 때 1.0이고 누적됩니다.
    Magnify { phase: Phase, scale: f32 },
    /// 화면에 닿은 손가락. 화면 전체를 트랙패드로 씁니다.
    Touch(Touch),
}

/// 입력을 Mac에 보낼 메시지로 바꿉니다.
///
/// - 스크롤과 확대는 마지막 포인터 위치에서 일어나므로, 포인터 위치를 기억해 함께 싣습니다.
/// - 트랙패드 스크롤이 끝나면 관성을 직접 만들어 [`Self::tick`]으로 이어 보냅니다. Mac은 관성을 클라이언트에게 맡깁니다.
/// - 손가락 터치는 상대 이동으로 커서를 옮기므로, 커서 위치를 이곳이 정합니다. 네이티브 계층은 [`Self::cursor`]로 받아 그립니다.
pub struct Input {
    keys: RecordKeys,
    // 직전 포인터 이벤트의 시각
    pointed: Instant,
    bounds: (u16, u16),
    // 레이아웃 픽셀이 Mac 포인트의 몇 배인지. 터치 스크롤의 이동량을 포인트로 바꿉니다.
    pixels_per_point: f32,
    at: (u16, u16),
    // 터치로 옮기는 커서의 소수 위치와 누른 버튼
    cursor: (f32, f32),
    buttons: u8,
    touchpad: Touchpad,
    // 관성 속도를 정하려고 남기는 최근 스크롤 이동
    recent: VecDeque<(Instant, (f32, f32))>,
    fling: Option<Fling>,
    // 정수 포인트로 보내고 남은 이동. 천천히 움직여도 이동을 잃지 않게 다음 이벤트에 더합니다.
    remainder: (f32, f32),
    // 확대 제스처에서 마지막으로 보낸 배율. 서버는 이벤트마다의 증분을 받습니다.
    scale: f32,
}

struct Fling {
    velocity: (f32, f32),
    last: Instant,
    began: bool,
}

impl Input {
    pub fn new(keys: RecordKeys) -> Self {
        Self {
            keys,
            pointed: Instant::now(),
            bounds: (u16::MAX, u16::MAX),
            pixels_per_point: 1.0,
            at: (0, 0),
            cursor: (0.0, 0.0),
            buttons: 0,
            touchpad: Touchpad::new(),
            recent: VecDeque::new(),
            fling: None,
            remainder: (0.0, 0.0),
            scale: 1.0,
        }
    }

    /// 레이아웃의 논리 크기와 픽셀 크기. 포인터 위치를 픽셀 크기 안으로 자릅니다.
    pub fn resize(&mut self, logical: (u16, u16), pixels: (u16, u16)) {
        self.bounds = pixels;
        self.pixels_per_point = f32::from(pixels.0) / f32::from(logical.0.max(1));
        self.at = self.clamp(self.at);
        self.cursor = (f32::from(self.at.0), f32::from(self.at.1));
    }

    /// 지금 커서 위치(레이아웃 픽셀)
    pub fn cursor(&self) -> (u16, u16) {
        self.at
    }

    pub fn handle(&mut self, event: Event, now: Instant) -> Vec<Vec<u8>> {
        match event {
            Event::Pointer { buttons, x, y } => {
                self.at = self.clamp((x, y));
                self.cursor = (f32::from(self.at.0), f32::from(self.at.1));
                vec![self.pointer(buttons, now)]
            }
            Event::Scroll { phase, delta } => self.scroll(phase, delta, now),
            Event::Wheel { lines } => {
                let delta = (lines.0 * POINTS_PER_LINE, lines.1 * POINTS_PER_LINE);
                let mut messages = Vec::from_iter(self.stop());
                messages.push(self.scroll_message(delta, None, None, false));
                messages
            }
            Event::Magnify { phase, scale } => vec![self.magnify(phase, scale)],
            Event::Touch(touch) => {
                let gestures = self.touchpad.handle(touch, now);
                gestures
                    .into_iter()
                    .flat_map(|gesture| self.gesture(gesture, now))
                    .collect()
            }
        }
    }

    fn gesture(&mut self, gesture: Gesture, now: Instant) -> Vec<Vec<u8>> {
        match gesture {
            Gesture::Move((dx, dy)) => {
                let max = (
                    f32::from(self.bounds.0.saturating_sub(1)),
                    f32::from(self.bounds.1.saturating_sub(1)),
                );
                self.cursor = (
                    (self.cursor.0 + dx).clamp(0.0, max.0),
                    (self.cursor.1 + dy).clamp(0.0, max.1),
                );
                // 범위 안으로 자른 양수라 `as`가 값을 잃지 않습니다.
                self.at = (self.cursor.0.round() as u16, self.cursor.1.round() as u16);
                vec![self.pointer(self.buttons, now)]
            }
            Gesture::Buttons(buttons) => {
                self.buttons = buttons;
                vec![self.pointer(buttons, now)]
            }
            Gesture::Scroll { phase, delta } => {
                let points = (delta.0 / self.pixels_per_point, delta.1 / self.pixels_per_point);
                self.scroll(phase, points, now)
            }
            Gesture::Magnify { phase, scale } => vec![self.magnify(phase, scale)],
        }
    }

    // 포인터 이벤트에는 직전 포인터 이벤트로부터 지난 시간을 싣습니다. 더블 클릭 판정에 쓰입니다.
    fn pointer(&mut self, buttons: u8, now: Instant) -> Vec<u8> {
        let elapsed_us = u32::try_from(now.saturating_duration_since(self.pointed).as_micros()).unwrap_or(u32::MAX);
        self.pointed = now;
        client::pointer(&self.keys, buttons, self.at.0, self.at.1, elapsed_us).to_vec()
    }

    /// 다음 관성 프레임을 보낼 시각
    pub fn deadline(&self) -> Option<Instant> {
        self.fling.as_ref().map(|fling| fling.last + FRAME)
    }

    /// 관성 프레임 하나를 보냅니다. 속도가 충분히 줄면 관성을 끝냅니다.
    pub fn tick(&mut self, now: Instant) -> Option<Vec<u8>> {
        let fling = self.fling.as_mut()?;
        let elapsed_ms = now.duration_since(fling.last).as_secs_f32() * 1000.0;
        let decay = DECELERATION_PER_MS.powf(elapsed_ms);
        fling.velocity = (fling.velocity.0 * decay, fling.velocity.1 * decay);
        fling.last = now;

        if speed(fling.velocity) < STOP_POINTS_PER_SEC {
            return self.stop();
        }

        let delta = (
            fling.velocity.0 * elapsed_ms / 1000.0,
            fling.velocity.1 * elapsed_ms / 1000.0,
        );
        let momentum = if std::mem::replace(&mut fling.began, true) {
            Momentum::Changed
        } else {
            Momentum::Began
        };

        Some(self.scroll_message(delta, None, Some(momentum), true))
    }

    fn scroll(&mut self, phase: Phase, delta: (f32, f32), now: Instant) -> Vec<Vec<u8>> {
        let mut messages = Vec::new();

        // 관성 중에 손가락이 다시 닿으면 관성을 끝내고 새 스크롤을 시작합니다.
        if phase == Phase::Began {
            messages.extend(self.stop());
            self.recent.clear();
            self.remainder = (0.0, 0.0);
        }

        self.recent.push_back((now, delta));

        while self
            .recent
            .front()
            .is_some_and(|&(at, _)| now.duration_since(at) > VELOCITY_WINDOW)
        {
            self.recent.pop_front();
        }

        messages.push(self.scroll_message(delta, Some(phase), None, true));

        if phase == Phase::Ended {
            let velocity = self.velocity(now);

            self.fling = (speed(velocity) >= FLING_POINTS_PER_SEC).then_some(Fling {
                velocity,
                last: now,
                began: false,
            });
        }

        messages
    }

    // 최근 이동의 합을 그 이동이 걸린 시간으로 나눈 속도(포인트/초)
    fn velocity(&self, now: Instant) -> (f32, f32) {
        let Some(&(first, _)) = self.recent.front() else {
            return (0.0, 0.0);
        };

        let seconds = now.duration_since(first).max(FRAME).as_secs_f32();
        let (x, y) = self
            .recent
            .iter()
            .fold((0.0, 0.0), |(x, y), (_, delta)| (x + delta.0, y + delta.1));
        (x / seconds, y / seconds)
    }

    // 진행 중인 관성을 끝냅니다. 아직 한 프레임도 보내지 않았으면 끝도 보내지 않습니다.
    fn stop(&mut self) -> Option<Vec<u8>> {
        let fling = self.fling.take()?;
        fling
            .began
            .then(|| self.scroll_message((0.0, 0.0), None, Some(Momentum::Ended), true))
    }

    // 시작할 때 배율은 1이고, 이후로는 직전 배율에 대한 증분을 보냅니다.
    fn magnify(&mut self, phase: Phase, scale: f32) -> Vec<u8> {
        let magnification = match phase {
            Phase::Began => {
                self.scale = 1.0;
                0.0
            }
            Phase::Changed => {
                let increment = f64::from(scale / self.scale - 1.0);
                self.scale = scale;
                increment
            }
            Phase::Ended | Phase::Cancelled => 0.0,
        };

        Magnify {
            magnification,
            phase,
            at: self.at,
        }
        .encode()
        .to_vec()
    }

    fn scroll_message(
        &mut self,
        delta: (f32, f32),
        phase: Option<Phase>,
        momentum: Option<Momentum>,
        continuous: bool,
    ) -> Vec<u8> {
        let total = (delta.0 + self.remainder.0, delta.1 + self.remainder.1);
        let points = (total.0.round(), total.1.round());
        self.remainder = (total.0 - points.0, total.1 - points.1);

        Scroll {
            // 실수에서 정수로의 `as`는 범위를 넘으면 끝에서 멈춥니다.
            points: (points.0 as i32, points.1 as i32),
            lines: (delta.0 / POINTS_PER_LINE, delta.1 / POINTS_PER_LINE),
            phase,
            momentum,
            continuous,
            at: self.at,
        }
        .encode()
        .to_vec()
    }

    fn clamp(&self, (x, y): (u16, u16)) -> (u16, u16) {
        (
            x.min(self.bounds.0.saturating_sub(1)),
            y.min(self.bounds.1.saturating_sub(1)),
        )
    }
}

fn speed((x, y): (f32, f32)) -> f32 {
    x.hypot(y)
}

#[cfg(test)]
mod tests {
    use super::{Event, FRAME, Input, Touch, TouchPhase};
    use crate::message::client;
    use crate::message::gesture::Phase;
    use crate::record::RecordKeys;
    use std::time::{Duration, Instant};

    fn input() -> Input {
        Input::new(RecordKeys {
            content_key: [5; 16],
            iv: [0; 16],
        })
    }

    // 스크롤 메시지의 (스크롤 단계, 관성 단계)
    fn phases(message: &[u8]) -> (u8, u8) {
        (message[41], message[45])
    }

    // 손가락을 움직이다 `pause` 뒤에 뗍니다. 뗀 시각을 반환합니다.
    fn swipe(input: &mut Input, start: Instant, step: (f32, f32), pause: Duration) -> Instant {
        input.handle(
            Event::Scroll {
                phase: Phase::Began,
                delta: (0.0, 0.0),
            },
            start,
        );

        let mut now = start;

        for _ in 0..5 {
            now += FRAME;
            input.handle(
                Event::Scroll {
                    phase: Phase::Changed,
                    delta: step,
                },
                now,
            );
        }

        now += pause;
        input.handle(
            Event::Scroll {
                phase: Phase::Ended,
                delta: (0.0, 0.0),
            },
            now,
        );
        now
    }

    #[test]
    fn fast_swipe_continues_with_momentum_until_it_stops() {
        let mut input = input();
        swipe(&mut input, Instant::now(), (0.0, 20.0), Duration::ZERO);

        let mut momentum = Vec::new();

        while let Some(deadline) = input.deadline() {
            momentum.extend(input.tick(deadline).map(|message| phases(&message)));
            assert!(momentum.len() < 1000, "관성이 끝나지 않습니다");
        }

        assert_eq!(momentum.first(), Some(&(0, 1)));
        assert_eq!(momentum.last(), Some(&(0, 3)));
        assert!(momentum[1..momentum.len() - 1].iter().all(|&phases| phases == (0, 2)));
    }

    #[test]
    fn swipe_that_paused_before_lifting_has_no_momentum() {
        let mut input = input();
        swipe(&mut input, Instant::now(), (0.0, 20.0), Duration::from_millis(300));
        assert!(input.deadline().is_none());
    }

    #[test]
    fn touching_again_ends_momentum_before_the_new_scroll() {
        let mut input = input();
        let now = swipe(&mut input, Instant::now(), (0.0, 20.0), Duration::ZERO);
        input.tick(now + FRAME);

        let messages = input.handle(
            Event::Scroll {
                phase: Phase::Began,
                delta: (0.0, 0.0),
            },
            now + FRAME * 2,
        );
        let phases: Vec<_> = messages.iter().map(|message| phases(message)).collect();
        assert_eq!(phases, [(0, 3), (1, 0)]);
        assert!(input.deadline().is_none());
    }

    #[test]
    fn wheel_has_no_phases_and_scrolls_at_the_pointer() {
        let mut input = input();
        input.resize((800, 600), (800, 600));
        input.handle(
            Event::Pointer {
                buttons: 0,
                x: 900,
                y: 10,
            },
            Instant::now(),
        );

        let messages = input.handle(Event::Wheel { lines: (0.0, 1.0) }, Instant::now());
        assert_eq!(messages.len(), 1);
        assert_eq!(phases(&messages[0]), (0, 0));
        assert_eq!(&messages[0][30..34], 10i32.to_be_bytes());
        assert_eq!(&messages[0][54..], [0x03, 0x1f, 0, 10]);
    }

    // 0.4포인트씩 세 번 움직이면 합한 1.2포인트에 가깝게, 정수 포인트 1을 보냅니다.
    #[test]
    fn slow_scroll_carries_fractions_instead_of_dropping_them() {
        let mut input = input();
        let now = Instant::now();
        input.handle(
            Event::Scroll {
                phase: Phase::Began,
                delta: (0.0, 0.0),
            },
            now,
        );

        let points: i32 = (0..3)
            .map(|_| {
                input.handle(
                    Event::Scroll {
                        phase: Phase::Changed,
                        delta: (0.0, 0.4),
                    },
                    now,
                )
            })
            .map(|messages| i32::from_be_bytes(messages[0][30..34].try_into().unwrap()))
            .sum();

        assert_eq!(points, 1);
    }

    #[test]
    fn magnify_sends_increments_whose_sign_is_the_direction() {
        let mut input = input();
        let now = Instant::now();
        let magnification = |message: &[u8]| f64::from_be_bytes(message[8..16].try_into().unwrap());

        let began = input.handle(
            Event::Magnify {
                phase: Phase::Began,
                scale: 1.0,
            },
            now,
        );
        let wider = input.handle(
            Event::Magnify {
                phase: Phase::Changed,
                scale: 1.1,
            },
            now,
        );
        let narrower = input.handle(
            Event::Magnify {
                phase: Phase::Changed,
                scale: 1.0,
            },
            now,
        );

        assert_eq!(magnification(&began[0]), 0.0);
        assert!((magnification(&wider[0]) - 0.1).abs() < 1e-6);
        assert!(magnification(&narrower[0]) < 0.0);
    }

    // 더블 클릭이 세션 시간과 관계없이 실제 간격으로 판단되도록, 시각은 직전 이벤트로부터의 마이크로초로 보냅니다.
    #[test]
    fn pointer_carries_microseconds_since_the_previous_event() {
        let mut input = input();
        input.resize((800, 600), (800, 600));
        let start = Instant::now() + Duration::from_secs(600);
        let click = |buttons| Event::Pointer { buttons, x: 10, y: 20 };
        input.handle(click(1), start);

        let keys = RecordKeys {
            content_key: [5; 16],
            iv: [0; 16],
        };
        assert_eq!(
            input.handle(click(0), start + Duration::from_millis(80)),
            vec![client::pointer(&keys, 0, 10, 20, 80_000).to_vec()]
        );
    }

    // 터치는 마지막 포인터 위치에서부터 상대 이동으로 커서를 옮기고, 화면 끝에서 멈춥니다.
    #[test]
    fn touch_moves_the_cursor_from_the_last_pointer_and_stops_at_the_edge() {
        let mut input = input();
        input.resize((400, 300), (800, 600));
        input.handle(
            Event::Pointer {
                buttons: 0,
                x: 100,
                y: 100,
            },
            Instant::now(),
        );

        let touch = |phase, at| Event::Touch(Touch { id: 1, phase, at });
        input.handle(touch(TouchPhase::Began, (0.0, 0.0)), Instant::now());
        assert_eq!(
            input
                .handle(touch(TouchPhase::Moved, (50.5, -20.0)), Instant::now())
                .len(),
            1
        );
        assert_eq!(input.cursor(), (151, 80));

        input.handle(touch(TouchPhase::Moved, (5000.0, -500.0)), Instant::now());
        assert_eq!(input.cursor(), (799, 0));
    }
}
