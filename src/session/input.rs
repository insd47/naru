use crate::message::client;
use crate::message::gesture::{Magnify, MomentumPhase, Phase, Scroll};
use crate::record::RecordKeys;
use std::time::{Duration, Instant};

// Points per line for trackpad scrolls, which carry no line count of their own. Apple's client uses about 10.
const POINTS_PER_LINE: f32 = 10.0;

// How far in layout pixels a press may be from the one before and still add to its click count, about 4 points on a
// Retina display. A trackpad or mouse often moves a pixel or two between the clicks of a double click.
const CLICK_SLOP: u16 = 8;

/// Input sent by the native layer.
#[derive(Debug, Clone, Copy)]
pub enum Event {
    /// Pointer position in layout pixels and buttons
    Pointer { buttons: u8, x: u16, y: u16 },
    /// Trackpad scroll while fingers touch. The delta is in points, and the direction follows CGEvent (positive is up
    /// and left).
    Scroll { phase: Phase, delta: (f32, f32) },
    /// Momentum after the fingers lift, with the same delta as scroll
    Momentum { phase: MomentumPhase, delta: (f32, f32) },
    /// Mouse wheel, as the client's system would scroll it: the distance in points and the same distance in lines, with
    /// the same direction as scroll. The Mac adds no acceleration.
    Wheel { points: (f32, f32), lines: (f32, f32) },
    /// Magnify. The scale starts at 1.0 and is cumulative.
    Magnify { phase: Phase, scale: f32 },
}

/// Turns input into messages for the Mac.
///
/// - Scroll and magnify happen at the last pointer position, so it remembers the position and includes it.
pub struct Input {
    keys: RecordKeys,
    // When the previous pointer event and key event were sent, and the pointer's buttons and position
    pointed: Instant,
    keyed: Instant,
    sent: Option<(u8, (u16, u16))>,
    bounds: (u16, u16),
    at: (u16, u16),
    // Movement left over after sending whole points, added to the next event so slow movement is not lost
    remainder: (f32, f32),
    // Last scale sent in the magnify gesture. The server takes per-event increments.
    scale: f32,
    clicks: Clicks,
}

impl Input {
    /// `double_click` is the client system's double-click interval.
    pub fn new(keys: RecordKeys, double_click: Duration) -> Self {
        Self {
            keys,
            pointed: Instant::now(),
            keyed: Instant::now(),
            sent: None,
            bounds: (u16::MAX, u16::MAX),
            at: (0, 0),
            remainder: (0.0, 0.0),
            scale: 1.0,
            clicks: Clicks {
                interval: double_click,
                last: None,
            },
        }
    }

    /// Sets the layout's pixel size. Pointer positions are clamped to it.
    pub fn resize(&mut self, pixels: (u16, u16)) {
        self.bounds = pixels;
        self.at = self.clamp(self.at);
    }

    /// The message for this input, or `None` when it changes nothing on the Mac.
    pub fn handle(&mut self, event: Event, now: Instant) -> Option<Vec<u8>> {
        match event {
            Event::Pointer { buttons, x, y } => {
                self.at = self.clamp((x, y));

                // iPadOS reports the unmoved pointer with every trackpad scroll event, and Firefox ends a scroll on a
                // mouse move.
                if self.sent == Some((buttons, self.at)) {
                    return None;
                }

                Some(self.pointer(buttons, now))
            }
            Event::Scroll { phase, delta } => {
                if matches!(phase, Phase::MayBegin | Phase::Began) {
                    self.remainder = (0.0, 0.0);
                }

                Some(self.scroll(delta, trackpad_lines(delta), Some(phase), None, true))
            }
            Event::Momentum { phase, delta } => {
                Some(self.scroll(delta, trackpad_lines(delta), None, Some(phase), true))
            }
            Event::Wheel { points, lines } => Some(self.scroll(points, lines, None, None, false)),
            Event::Magnify { phase, scale } => Some(self.magnify(phase, scale)),
        }
    }

    /// A key with its macOS virtual key code. Key events carry the time since the previous key event.
    pub fn key(&mut self, keysym: u32, keycode: u16, down: bool, now: Instant) -> Vec<u8> {
        let elapsed_us = u32::try_from(now.saturating_duration_since(self.keyed).as_micros()).unwrap_or(u32::MAX);
        self.keyed = now;
        client::key_event(&self.keys, keysym, down, keycode, elapsed_us).to_vec()
    }

    // Pointer events carry the time since the previous pointer event, and the click count.
    fn pointer(&mut self, buttons: u8, now: Instant) -> Vec<u8> {
        let elapsed_us = u32::try_from(now.saturating_duration_since(self.pointed).as_micros()).unwrap_or(u32::MAX);
        let held = self.sent.map_or(0, |(buttons, _)| buttons);
        let clicks = self.clicks.count(held, buttons, self.at, now);
        self.pointed = now;
        self.sent = Some((buttons, self.at));
        client::pointer(&self.keys, buttons, clicks, self.at.0, self.at.1, elapsed_us).to_vec()
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
            Phase::MayBegin | Phase::Ended | Phase::Cancelled => 0.0,
        };

        Magnify {
            magnification,
            phase,
            at: self.at,
        }
        .encode()
        .to_vec()
    }

    fn scroll(
        &mut self,
        delta: (f32, f32),
        lines: (f32, f32),
        phase: Option<Phase>,
        momentum: Option<MomentumPhase>,
        continuous: bool,
    ) -> Vec<u8> {
        // Only continuous scrolls carry fractions over; a wheel sends its own distance as given.
        let carried = if continuous { self.remainder } else { (0.0, 0.0) };
        let total = (delta.0 + carried.0, delta.1 + carried.1);
        let points = (total.0.round(), total.1.round());

        if continuous {
            self.remainder = (total.0 - points.0, total.1 - points.1);
        }

        Scroll {
            // Float-to-int `as` saturates out of range.
            points: (points.0 as i32, points.1 as i32),
            lines,
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

/// Counts clicks as the client's system does, since the Mac takes the count as is, as it does from Apple's client.
///
/// - A press adds to the count of the one before when it is of the same button, within the double-click interval of
///   that one's release, as Android measures its double-tap timeout, and within `CLICK_SLOP` of its position.
///   Otherwise it starts at 1.
/// - Moves while a button is held and the release keep the press's count, and moves with no button held have none.
struct Clicks {
    interval: Duration,
    last: Option<Click>,
}

// The latest press, and when it was released
struct Click {
    button: u8,
    at: (u16, u16),
    count: u8,
    released: Option<Instant>,
}

impl Clicks {
    // The count for a pointer event with `buttons` at `at`, after one with `held`
    fn count(&mut self, held: u8, buttons: u8, at: (u16, u16), now: Instant) -> u8 {
        let pressed = buttons & !held;

        if pressed == 0 {
            let Some(last) = self.last.as_mut().filter(|_| buttons | held != 0) else {
                return 0;
            };

            if held & last.button != 0 && buttons & last.button == 0 {
                last.released = Some(now);
            }

            return last.count;
        }

        let repeated = self.last.as_ref().filter(|last| {
            last.button == pressed
                && last
                    .released
                    .is_some_and(|released| now.saturating_duration_since(released) <= self.interval)
                && last.at.0.abs_diff(at.0) <= CLICK_SLOP
                && last.at.1.abs_diff(at.1) <= CLICK_SLOP
        });
        let count = repeated.map_or(1, |last| last.count.saturating_add(1));

        self.last = Some(Click {
            button: pressed,
            at,
            count,
            released: None,
        });
        count
    }
}

// A trackpad scroll in lines, for apps that scroll line by line
fn trackpad_lines((x, y): (f32, f32)) -> (f32, f32) {
    (x / POINTS_PER_LINE, y / POINTS_PER_LINE)
}

#[cfg(test)]
mod tests {
    use super::{Event, Input};
    use crate::message::client;
    use crate::message::gesture::{MomentumPhase, Phase};
    use crate::record::RecordKeys;
    use std::time::{Duration, Instant};

    const DOUBLE_CLICK: Duration = Duration::from_millis(500);

    fn input() -> Input {
        Input::new(
            RecordKeys {
                content_key: [5; 16],
                iv: [0; 16],
            },
            DOUBLE_CLICK,
        )
    }

    // The click count of each pointer event, from its flags, for (buttons, x, milliseconds since the first event)
    fn clicks(events: &[(u8, u16, u64)]) -> Vec<u8> {
        let mut input = input();
        input.resize((800, 600));
        let start = Instant::now();

        events
            .iter()
            .map(|&(buttons, x, ms)| {
                let message = input.handle(Event::Pointer { buttons, x, y: 20 }, start + Duration::from_millis(ms));
                message.unwrap()[1] >> 1
            })
            .collect()
    }

    // (scroll phase, momentum phase) of a scroll message
    fn phases(message: &[u8]) -> (u8, u8) {
        (message[41], message[45])
    }

    fn scroll(phase: Phase, delta: (f32, f32)) -> Event {
        Event::Scroll { phase, delta }
    }

    // A resting finger that lifts without moving reaches the Mac as may-begin and cancelled, as from Apple's client.
    #[test]
    fn resting_finger_sends_may_begin_then_cancelled() {
        let mut input = input();
        let now = Instant::now();
        let rested = input.handle(scroll(Phase::MayBegin, (0.0, 0.0)), now).unwrap();
        let lifted = input.handle(scroll(Phase::Cancelled, (0.0, 0.0)), now).unwrap();
        assert_eq!((phases(&rested), phases(&lifted)), ((128, 0), (8, 0)));
    }

    #[test]
    fn momentum_carries_its_own_phase_without_a_scroll_phase() {
        let mut input = input();
        let now = Instant::now();
        let sent: Vec<_> = [MomentumPhase::Began, MomentumPhase::Changed, MomentumPhase::Ended]
            .into_iter()
            .map(|phase| {
                phases(
                    &input
                        .handle(
                            Event::Momentum {
                                phase,
                                delta: (0.0, 12.0),
                            },
                            now,
                        )
                        .unwrap(),
                )
            })
            .collect();
        assert_eq!(sent, [(0, 1), (0, 2), (0, 3)]);
    }

    // A trackpad fraction left over must not shift a wheel notch, which carries the client's own distance.
    #[test]
    fn wheel_ignores_the_trackpad_remainder() {
        let mut input = input();
        let now = Instant::now();
        input.handle(
            Event::Scroll {
                phase: Phase::Began,
                delta: (0.0, 0.0),
            },
            now,
        );
        input.handle(
            Event::Scroll {
                phase: Phase::Changed,
                delta: (0.0, 0.4),
            },
            now,
        );

        let messages = input.handle(
            Event::Wheel {
                points: (0.0, 30.3),
                lines: (0.0, 3.0),
            },
            now,
        );
        assert_eq!(&messages.unwrap()[30..34], 30i32.to_be_bytes());
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

        let message = input
            .handle(
                Event::Wheel {
                    points: (0.0, 100.0),
                    lines: (0.0, 3.0),
                },
                Instant::now(),
            )
            .unwrap();
        assert_eq!(phases(&message), (0, 0));
        // The client's distance and line count go out as given, not derived from each other.
        assert_eq!(&message[10..12], 3i16.to_be_bytes());
        assert_eq!(&message[30..34], 100i32.to_be_bytes());
        assert_eq!(&message[54..], [0x03, 0x1f, 0, 10]);
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
            .map(|message| i32::from_be_bytes(message.unwrap()[30..34].try_into().unwrap()))
            .sum();

        assert_eq!(points, 1);
    }

    #[test]
    fn magnify_sends_increments_whose_sign_is_the_direction() {
        let mut input = input();
        let now = Instant::now();
        let magnification = |message: Option<Vec<u8>>| f64::from_be_bytes(message.unwrap()[8..16].try_into().unwrap());

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

        assert_eq!(magnification(began), 0.0);
        assert!((magnification(wider) - 0.1).abs() < 1e-6);
        assert!(magnification(narrower) < 0.0);
    }

    // Time is sent as microseconds since the previous event, so double clicks are judged by the real interval
    // regardless of session time.
    #[test]
    fn pointer_carries_microseconds_since_the_previous_event() {
        let mut input = input();
        input.resize((800, 600));
        let start = Instant::now() + Duration::from_secs(600);
        let click = |buttons| Event::Pointer { buttons, x: 10, y: 20 };
        input.handle(click(1), start);

        let keys = RecordKeys {
            content_key: [5; 16],
            iv: [0; 16],
        };
        assert_eq!(
            input.handle(click(0), start + Duration::from_millis(80)),
            Some(client::pointer(&keys, 0, 1, 10, 20, 80_000).to_vec())
        );
    }

    // The pointer moving a pixel between the clicks still makes a double click, its release included, and a third
    // click within the interval a triple click.
    #[test]
    fn clicks_count_up_within_the_interval_and_a_few_pixels() {
        let events = [
            (0, 10, 0),
            (1, 10, 0),
            (0, 10, 90),
            (0, 11, 150),
            (1, 11, 200),
            (0, 11, 290),
            (1, 12, 500),
        ];
        assert_eq!(clicks(&events), [0, 1, 1, 0, 2, 2, 3]);
    }

    #[test]
    fn clicks_start_over_when_slow_far_or_of_another_button() {
        let slow = [(1, 10, 0), (0, 10, 90), (1, 10, 600)];
        let far = [(1, 10, 0), (0, 10, 90), (1, 30, 200)];
        let other = [(1, 10, 0), (0, 10, 90), (2, 10, 200), (0, 10, 290)];
        assert_eq!(clicks(&slow), [1, 1, 1]);
        assert_eq!(clicks(&far), [1, 1, 1]);
        assert_eq!(clicks(&other), [1, 1, 1, 1]);
    }

    // A drag and its release keep the count of the press that began it.
    #[test]
    fn a_held_button_keeps_its_count_while_it_moves() {
        let events = [
            (1, 10, 0),
            (0, 10, 90),
            (1, 10, 200),
            (1, 40, 300),
            (0, 80, 400),
            (0, 90, 500),
        ];
        assert_eq!(clicks(&events), [1, 1, 2, 2, 2, 0]);
    }

    #[test]
    fn unchanged_pointer_is_not_resent() {
        let mut input = input();
        input.resize((800, 600));
        let point = |buttons| Event::Pointer { buttons, x: 10, y: 20 };

        assert!(input.handle(point(0), Instant::now()).is_some());
        assert!(input.handle(point(0), Instant::now()).is_none());
        assert!(input.handle(point(1), Instant::now()).is_some());
    }
}
