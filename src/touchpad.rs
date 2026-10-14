//! Turns fingers on the screen into Mac trackpad gestures, for clients whose screen has no pointer of its own.
//!
//! - It knows nothing of the session or the layout. The client keeps the cursor it moves, clamps it to the screen,
//!   converts scroll distances to points, and sends the results through the session like any other pointer and scroll.
//! - Momentum after a two-finger scroll comes from the client's system, as it does for a hardware trackpad.
//! - Positions are in layout pixels, which the tap and slop distances below are tuned for.
use crate::message::gesture::Phase;
use std::collections::HashMap;
use std::time::{Duration, Instant};

// Lifting within this time after moving less than this distance is a tap. Distances are in layout pixels.
const TAP: Duration = Duration::from_millis(250);
const TAP_SLOP: f32 = 20.0;

// Touching down again within this time after a tap starts a drag with the button held. Lifting right away makes a
// second click, which the Mac takes as a double click.
const DRAG_AFTER_TAP: Duration = Duration::from_millis(300);

// Once two fingers first move this far, the gesture is fixed as magnify or scroll until all fingers lift.
const PINCH_SLOP: f32 = 20.0;
const SCROLL_SLOP: f32 = 10.0;

/// The phase of a single finger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TouchPhase {
    Began,
    Moved,
    Ended,
    Cancelled,
}

/// A single finger touch. The position is in layout pixels and is not clamped to the screen.
#[derive(Debug, Clone, Copy)]
pub struct Touch {
    pub id: u32,
    pub phase: TouchPhase,
    pub at: (f32, f32),
}

/// A trackpad gesture derived from touches. Distances are in layout pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    /// Moves the cursor by this much, in layout pixels.
    Move((f32, f32)),
    /// Changes the button state. Bits match pointer events (left 1, right 2).
    Buttons(u8),
    /// Trackpad scroll. The delta is the fingers' movement, positive down and right.
    Scroll { phase: Phase, delta: (f32, f32) },
    /// Magnify. The scale starts at 1.0 and is cumulative.
    Magnify { phase: Phase, scale: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    /// One finger. Moves the cursor and checks for a tap on lift.
    Point,
    /// One finger touching down right after a tap. Moves with the left button held.
    Drag,
    /// Two fingers that have not moved yet. Lifting is a right click.
    Pair,
    Scroll,
    Pinch,
    /// Three or more fingers. These are system gestures, so nothing is sent until all fingers lift.
    Ignore,
}

/// Turns finger touches into trackpad gestures. The whole screen is one trackpad.
///
/// - One finger moves the cursor and taps to left-click. Touching down right after a tap and moving drags.
/// - Two fingers tap to right-click and move to scroll or magnify. Lifting one finger keeps the gesture going until all
///   fingers lift.
/// - Three or more fingers are iPadOS or Android system gestures, so the current gesture ends and they are ignored.
pub struct Touchpad {
    fingers: HashMap<u32, (f32, f32)>,
    mode: Option<Mode>,
    // When the first finger touched down, and how far it has moved since
    began: Instant,
    travel: f32,
    // Previous center of the two-finger gesture, and the distance when both fingers touched down
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
            Some(Mode::Pinch) if self.fingers.len() == 2 => vec![Gesture::Magnify {
                phase: Phase::Changed,
                scale: self.spread() / self.base_spread,
            }],
            _ => Vec::new(),
        }
    }

    fn end(&mut self, touch: Touch, now: Instant) -> Vec<Gesture> {
        if self.fingers.remove(&touch.id).is_none() {
            return Vec::new();
        }

        // With a finger remaining, the gesture continues. Move the reference so the center does not jump to the
        // remaining finger.
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

    // Decides between magnify and scroll from how the two fingers first moved.
    fn classify(&mut self) -> Vec<Gesture> {
        let center = self.centroid();
        let moved = (center.0 - self.origin.0).hypot(center.1 - self.origin.1);
        self.travel = self.travel.max(moved);

        if self.base_spread > 0.0 && (self.spread() - self.base_spread).abs() > PINCH_SLOP {
            self.mode = Some(Mode::Pinch);
            return vec![Gesture::Magnify {
                phase: Phase::Began,
                scale: 1.0,
            }];
        }

        // The first scroll carries the movement so far, as the Mac's own scrolls begin.
        if moved > SCROLL_SLOP {
            self.mode = Some(Mode::Scroll);
            return self.scroll(Phase::Began);
        }

        Vec::new()
    }

    // Scrolls by how far the center moved. Moving fingers down is positive, the same direction as iPad trackpad
    // scrolling.
    fn scroll(&mut self, phase: Phase) -> Vec<Gesture> {
        let center = self.centroid();
        let delta = (center.0 - self.origin.0, center.1 - self.origin.1);
        self.origin = center;
        vec![Gesture::Scroll { phase, delta }]
    }

    // Ends the current gesture, releasing held buttons and ending any scroll or magnify.
    fn finish(&mut self) -> Vec<Gesture> {
        match self.mode {
            Some(Mode::Drag) => vec![Gesture::Buttons(0)],
            Some(Mode::Scroll) => vec![Gesture::Scroll {
                phase: Phase::Ended,
                delta: (0.0, 0.0),
            }],
            Some(Mode::Pinch) => vec![Gesture::Magnify {
                phase: Phase::Ended,
                scale: self.spread() / self.base_spread,
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

    // Distance between two fingers, or 0 when there are not exactly two.
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

    // Lifting one finger first neither moves the cursor with the remaining finger nor makes the scroll jump from a
    // shifted center.
    #[test]
    fn two_fingers_scroll_until_both_lift() {
        let mut fingers = Fingers::new();
        fingers.touch(1, Began, (100.0, 100.0));
        fingers.touch(2, Began, (160.0, 100.0));

        assert_eq!(
            fingers.touch(1, Moved, (100.0, 140.0)),
            [Gesture::Scroll {
                phase: Phase::Began,
                delta: (0.0, 20.0)
            }]
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
