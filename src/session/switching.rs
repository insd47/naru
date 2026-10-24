//! Switches the Mac's input source before the keys typed in the new language reach it.
//!
//! - The agent applies a requested source only at the next key event, and when that event is a typed key it often reads
//!   the old source's layout and keeps it, typing English or separate jamo until a text field takes focus again. Apple's
//!   client applies the source at the release of its switch key instead, before the user types.
//! - So the request follows a release of the agent's source, which Apple's client sends when "Sync Keyboard Language"
//!   turns off and which makes that state far rarer, and is applied at once by a key that types nothing.
//! - That key is released only after the switch, since the agent selects the source again at every key until the first
//!   selection finishes. Keys typed meanwhile wait for the Mac to report the new source, a few milliseconds, or for a
//!   short timeout when it never comes.
use crate::message::client;
use std::time::{Duration, Instant};

// How long keys wait for the Mac to report the new source
const WAIT: Duration = Duration::from_millis(150);

// X11 VoidSymbol, a key with no character
const NOTHING: u32 = 0xff_ffff;

/// A source the Mac was asked to switch to, and the messages waiting for the switch
pub struct Switching {
    id: String,
    deadline: Instant,
    held: Vec<Vec<u8>>,
}

impl Switching {
    /// Asks the Mac for the source and returns the messages to send now. Messages still waiting for an earlier request
    /// keep waiting, now for this one, and the key that types nothing stays pressed from it.
    pub fn start(id: &str, previous: Option<Switching>, now: Instant) -> (Self, Vec<Vec<u8>>) {
        let mut messages = vec![client::input_source(""), client::input_source(id)];
        let held = match previous {
            Some(previous) => previous.held,
            None => {
                messages.push(client::key(NOTHING, true).to_vec());
                vec![client::key(NOTHING, false).to_vec()]
            }
        };

        let switching = Self {
            id: id.to_owned(),
            deadline: now + WAIT,
            held,
        };
        (switching, messages)
    }

    pub fn hold(&mut self, message: Vec<u8>) {
        self.held.push(message);
    }

    /// Whether the Mac reported the requested source
    pub fn done(&self, reported: &str) -> bool {
        self.id == reported
    }

    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// The release of the key that types nothing, then the held messages in order
    pub fn release(self) -> Vec<Vec<u8>> {
        self.held
    }
}

#[cfg(test)]
mod tests {
    use super::Switching;
    use crate::message::client;
    use std::time::Instant;

    const KOREAN: &str = "com.apple.inputmethod.Korean.2SetKorean";
    const ABC: &str = "com.apple.keylayout.ABC";

    // The agent's source is released, the new one asked for, and a key that types nothing pressed to apply it. Its
    // release waits for the switch.
    #[test]
    fn request_is_released_asked_and_applied() {
        let (switching, messages) = Switching::start(ABC, None, Instant::now());
        assert_eq!(
            messages,
            [
                client::input_source(""),
                client::input_source(ABC),
                client::key(0xff_ffff, true).to_vec()
            ]
        );
        assert_eq!(switching.release(), [client::key(0xff_ffff, false).to_vec()]);
    }

    #[test]
    fn keys_wait_for_the_latest_request_in_order() {
        let now = Instant::now();
        let (mut switching, _) = Switching::start(ABC, None, now);
        switching.hold(client::key(b'k'.into(), true).to_vec());

        let (mut switching, messages) = Switching::start(KOREAN, Some(switching), now);
        switching.hold(client::key(b'k'.into(), false).to_vec());

        assert_eq!(messages, [client::input_source(""), client::input_source(KOREAN)]);
        assert!(!switching.done(ABC));
        assert!(switching.done(KOREAN));
        assert_eq!(
            switching.release(),
            [
                client::key(0xff_ffff, false).to_vec(),
                client::key(b'k'.into(), true).to_vec(),
                client::key(b'k'.into(), false).to_vec()
            ]
        );
    }
}
