//! Keeps keys in order while the Mac switches its input source.
//!
//! - The agent selects a requested source only at the next key, and the frontmost app's input method follows a moment
//!   later, so a key sent right behind the request still goes to the old input method. Apple's client types correctly
//!   from the first key, so it waits for the Mac's report too.
//! - A lone Control press after the request makes the agent switch at once, and keys wait for the report of the new
//!   source, a few milliseconds on a local network, or for a short timeout when it never comes.
use crate::message::client;
use std::time::{Duration, Instant};

// How long keys wait for the Mac to report the new source
const WAIT: Duration = Duration::from_millis(150);

const CONTROL: u32 = 0xffe3;

/// A source the Mac was asked to select, and the keys waiting for it
pub struct Switching {
    id: String,
    deadline: Instant,
    keys: Vec<[u8; 8]>,
}

impl Switching {
    /// Asks the Mac for the source and returns the messages to send now. Keys still waiting for an earlier request keep
    /// waiting, now for this one.
    pub fn start(id: &str, previous: Option<Switching>, now: Instant) -> (Self, Vec<Vec<u8>>) {
        let switching = Self {
            id: id.to_owned(),
            deadline: now + WAIT,
            keys: previous.map(|previous| previous.keys).unwrap_or_default(),
        };
        let messages = vec![
            client::input_source(id),
            client::key(CONTROL, true).to_vec(),
            client::key(CONTROL, false).to_vec(),
        ];
        (switching, messages)
    }

    pub fn hold(&mut self, key: [u8; 8]) {
        self.keys.push(key);
    }

    /// Whether the Mac reported the requested source
    pub fn done(&self, reported: &str) -> bool {
        self.id == reported
    }

    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// The held keys, in the order they were pressed
    pub fn release(self) -> Vec<[u8; 8]> {
        self.keys
    }
}

#[cfg(test)]
mod tests {
    use super::Switching;
    use crate::message::client;
    use std::time::Instant;

    const KOREAN: &str = "com.apple.inputmethod.Korean.2SetKorean";
    const ABC: &str = "com.apple.keylayout.ABC";

    // The request goes out first, then the Control press that makes the agent apply it.
    #[test]
    fn request_is_followed_by_a_lone_control_press() {
        let (_, messages) = Switching::start(ABC, None, Instant::now());
        assert_eq!(messages[0], client::input_source(ABC));
        assert_eq!(
            messages[1..],
            [client::key(0xffe3, true).to_vec(), client::key(0xffe3, false).to_vec()]
        );
    }

    #[test]
    fn keys_wait_for_the_latest_request_in_order() {
        let now = Instant::now();
        let (mut switching, _) = Switching::start(ABC, None, now);
        switching.hold(client::key(b'k'.into(), true));

        let (mut switching, _) = Switching::start(KOREAN, Some(switching), now);
        switching.hold(client::key(b'k'.into(), false));

        assert!(!switching.done(ABC));
        assert!(switching.done(KOREAN));
        assert_eq!(
            switching.release(),
            [client::key(b'k'.into(), true), client::key(b'k'.into(), false)]
        );
    }
}
