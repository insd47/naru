use crate::message::client;
use crate::message::gesture::{Magnify, Momentum, Phase, Scroll};
use crate::record::RecordKeys;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use touch::{Gesture, Touchpad};
pub use touch::{Touch, TouchPhase};

mod touch;

// Points per line. Apple's client uses about 10 for trackpad scrolls, and one wheel notch scrolls the same amount.
const POINTS_PER_LINE: f32 = 10.0;

// Momentum scrolling. The velocity comes from the movement just before the fingers lift and decays at UIScrollView's default deceleration (×0.998 per ms).
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);
const FRAME: Duration = Duration::from_millis(16);
const DECELERATION_PER_MS: f32 = 0.998;
const FLING_POINTS_PER_SEC: f32 = 100.0;
const STOP_POINTS_PER_SEC: f32 = 10.0;

/// Input sent by the native layer.
#[derive(Debug, Clone, Copy)]
pub enum Event {
    /// Pointer position in layout pixels and buttons
    Pointer { buttons: u8, x: u16, y: u16 },
    /// Trackpad scroll. The delta is in points, and the direction follows CGEvent (positive is up and left).
    Scroll { phase: Phase, delta: (f32, f32) },
    /// Mouse wheel, in notches, with the same direction as scroll
    Wheel { lines: (f32, f32) },
    /// Magnify. The scale starts at 1.0 and is cumulative.
    Magnify { phase: Phase, scale: f32 },
    /// A finger on the screen. The whole screen acts as a trackpad.
    Touch(Touch),
}

/// Turns input into messages for the Mac.
///
/// - Scroll and magnify happen at the last pointer position, so it remembers the position and includes it.
/// - When a trackpad scroll ends, it generates momentum itself and continues it through [`Self::tick`], because the Mac leaves momentum to the client.
/// - Finger touches move the cursor relatively, so this owns the cursor position. The native layer reads it with [`Self::cursor`] and draws it.
pub struct Input {
    keys: RecordKeys,
    // When the previous pointer event was sent, and its buttons and position
    pointed: Instant,
    sent: Option<(u8, (u16, u16))>,
    bounds: (u16, u16),
    // Layout pixels per Mac point, used to convert touch scroll distances to points
    pixels_per_point: f32,
    at: (u16, u16),
    // Fractional position of the touch-driven cursor and the pressed buttons
    cursor: (f32, f32),
    buttons: u8,
    touchpad: Touchpad,
    // Recent scroll movements kept to compute momentum velocity
    recent: VecDeque<(Instant, (f32, f32))>,
    fling: Option<Fling>,
    // Movement left over after sending whole points, added to the next event so slow movement is not lost
    remainder: (f32, f32),
    // Last scale sent in the magnify gesture. The server takes per-event increments.
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
            sent: None,
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

    /// Sets the layout's logical and pixel sizes. Pointer positions are clamped to the pixel size.
    pub fn resize(&mut self, logical: (u16, u16), pixels: (u16, u16)) {
        self.bounds = pixels;
        self.pixels_per_point = f32::from(pixels.0) / f32::from(logical.0.max(1));
        self.at = self.clamp(self.at);
        self.cursor = (f32::from(self.at.0), f32::from(self.at.1));
    }

    /// Current cursor position in layout pixels
    pub fn cursor(&self) -> (u16, u16) {
        self.at
    }

    pub fn handle(&mut self, event: Event, now: Instant) -> Vec<Vec<u8>> {
        match event {
            Event::Pointer { buttons, x, y } => {
                self.at = self.clamp((x, y));
                self.cursor = (f32::from(self.at.0), f32::from(self.at.1));

                // iPadOS reports the unmoved pointer with every trackpad scroll event, and Firefox ends a scroll on a mouse move.
                if self.sent == Some((buttons, self.at)) {
                    return Vec::new();
                }

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
                // Clamped and non-negative, so `as` loses nothing.
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

    // Pointer events carry the time since the previous pointer event, which decides double clicks.
    fn pointer(&mut self, buttons: u8, now: Instant) -> Vec<u8> {
        let elapsed_us = u32::try_from(now.saturating_duration_since(self.pointed).as_micros()).unwrap_or(u32::MAX);
        self.pointed = now;
        self.sent = Some((buttons, self.at));
        client::pointer(&self.keys, buttons, self.at.0, self.at.1, elapsed_us).to_vec()
    }

    /// When to send the next momentum frame
    pub fn deadline(&self) -> Option<Instant> {
        self.fling.as_ref().map(|fling| fling.last + FRAME)
    }

    /// Sends one momentum frame, ending momentum once the velocity has decayed enough.
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

        // Fingers touching down during momentum end it and start a new scroll.
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

    // Velocity in points per second: recent movement divided by the time it took
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

    // Ends momentum in progress. If no frame went out yet, no end is sent either.
    fn stop(&mut self) -> Option<Vec<u8>> {
        let fling = self.fling.take()?;
        fling
            .began
            .then(|| self.scroll_message((0.0, 0.0), None, Some(Momentum::Ended), true))
    }

    // The scale starts at 1, and each event sends the increment over the previous scale.
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
            // Float-to-int `as` saturates out of range.
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

    // (scroll phase, momentum phase) of a scroll message
    fn phases(message: &[u8]) -> (u8, u8) {
        (message[41], message[45])
    }

    // Moves a finger, then lifts it after `pause`. Returns when it lifted.
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
            assert!(momentum.len() < 1000, "momentum never ends");
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

    // Three 0.4-point moves send one whole point, close to their 1.2-point sum.
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

    // Time is sent as microseconds since the previous event, so double clicks are judged by the real interval regardless of session time.
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

    #[test]
    fn unchanged_pointer_is_not_resent() {
        let mut input = input();
        input.resize((800, 600), (800, 600));
        let point = |buttons| Event::Pointer { buttons, x: 10, y: 20 };

        assert_eq!(input.handle(point(0), Instant::now()).len(), 1);
        assert!(input.handle(point(0), Instant::now()).is_empty());
        assert_eq!(input.handle(point(1), Instant::now()).len(), 1);
    }

    // Touch moves the cursor relatively from the last pointer position and stops at the screen edge.
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
