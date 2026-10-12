use crate::message::client;
use crate::message::gesture::{Magnify, Momentum, Phase, Scroll};
use crate::record::RecordKeys;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

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
}

/// 입력을 Mac에 보낼 메시지로 바꿉니다.
///
/// - 스크롤과 확대는 마지막 포인터 위치에서 일어나므로, 포인터 위치를 기억해 함께 싣습니다.
/// - 트랙패드 스크롤이 끝나면 관성을 직접 만들어 [`Self::tick`]으로 이어 보냅니다. Mac은 관성을 클라이언트에게 맡깁니다.
pub struct Input {
    keys: RecordKeys,
    started: Instant,
    bounds: (u16, u16),
    at: (u16, u16),
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
            started: Instant::now(),
            bounds: (u16::MAX, u16::MAX),
            at: (0, 0),
            recent: VecDeque::new(),
            fling: None,
            remainder: (0.0, 0.0),
            scale: 1.0,
        }
    }

    /// 레이아웃의 픽셀 크기. 포인터 위치를 이 안으로 자릅니다.
    pub fn resize(&mut self, pixels: (u16, u16)) {
        self.bounds = pixels;
        self.at = self.clamp(self.at);
    }

    pub fn handle(&mut self, event: Event, now: Instant) -> Vec<Vec<u8>> {
        match event {
            Event::Pointer { buttons, x, y } => {
                self.at = self.clamp((x, y));

                // 시각이 있어야 같은 위치와 버튼의 이벤트가 이어져도 서버가 중복으로 버리지 않습니다.
                let time_ms = u32::try_from(now.duration_since(self.started).as_millis()).unwrap_or(u32::MAX);
                vec![client::pointer(&self.keys, buttons, self.at.0, self.at.1, time_ms).to_vec()]
            }
            Event::Scroll { phase, delta } => self.scroll(phase, delta, now),
            Event::Wheel { lines } => {
                let delta = (lines.0 * POINTS_PER_LINE, lines.1 * POINTS_PER_LINE);
                let mut messages = Vec::from_iter(self.stop());
                messages.push(self.scroll_message(delta, None, None, false));
                messages
            }
            Event::Magnify { phase, scale } => vec![self.magnify(phase, scale)],
        }
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
    use super::{Event, FRAME, Input};
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
        input.resize((800, 600));
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
}
