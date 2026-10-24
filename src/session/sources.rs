//! The Mac's input source as the client's input language and "Sync Keyboard Language" decide it, and whether keys go
//! with their physical key.
//!
//! - With the option on, the Mac is asked for the source of the client's language whenever that changes, and once it
//!   first reports its own source, which picks the mode of a language (Sebeolsik, Kana typing). A source already asked
//!   for is not asked again, so one the Mac changed to by itself, as when another window took focus, is left alone
//!   until the client's language changes, as Apple's client leaves it: asking right after such a change can leave the
//!   agent typing with the other source's layout.
//! - Turned off, the Mac keeps its own source again, as Apple's client lets it; the agent would otherwise undo a switch
//!   made on the Mac at every key.
//! - Like Apple's client, a key goes with its physical key only while the Mac is on the source of the client's
//!   language, where that key types the same, and otherwise the Mac types the keysym the client chose. With the option
//!   on, the source last asked for counts as the Mac's, since keys wait for the switch to it; with it off, the source
//!   the Mac reported does.
use super::switching::Switching;
use crate::keyboard;
use crate::message::{InputSource, client};
use std::time::Instant;

/// Lives as long as the session. What it asked of a Mac is forgotten when a connection starts.
#[derive(Default)]
pub struct Sources {
    // The client's input language, and whether the Mac's source follows it
    language: Option<String>,
    syncs: bool,
    // The Mac's latest source, the one last asked for, and a switch in progress
    reported: Option<InputSource>,
    asked: Option<String>,
    switching: Option<Switching>,
}

impl Sources {
    /// A new connection, to a Mac that has been asked nothing yet
    pub fn connect(&mut self) {
        self.reported = None;
        self.asked = None;
        self.switching = None;
    }

    /// The client's input language changed. Returns the messages to send.
    pub fn language(&mut self, language: String, now: Instant) -> Vec<Vec<u8>> {
        self.language = Some(language);
        self.follow(now)
    }

    /// "Sync Keyboard Language" was turned on or off. Returns the messages to send.
    pub fn sync(&mut self, on: bool, now: Instant) -> Vec<Vec<u8>> {
        self.syncs = on;

        if on {
            return self.follow(now);
        }

        // Keys waiting for a switch go before the release.
        let Some(_) = self.asked.take() else {
            return Vec::new();
        };

        let mut messages = self.switched();
        messages.push(client::input_source(""));
        messages
    }

    /// The Mac reported its source. Returns the messages to send.
    pub fn reported(&mut self, source: InputSource, now: Instant) -> Vec<Vec<u8>> {
        let first = self.reported.is_none();
        let done = self
            .switching
            .as_ref()
            .is_some_and(|switching| switching.done(&source.id));
        self.reported = Some(source);

        let mut messages = if done { self.switched() } else { Vec::new() };

        if first {
            messages.extend(self.follow(now));
        }

        messages
    }

    /// The macOS virtual key code to send with a key, or `None` to send its keysym alone
    pub fn keycode(&self, keysym: u32, usage: Option<u16>) -> Option<u16> {
        let target = self.target()?;
        let mac = if self.syncs {
            self.asked.as_deref()
        } else {
            self.reported.as_ref().map(|source| source.id.as_str())
        };

        (mac == Some(target.as_str()))
            .then(|| keyboard::keycode(keysym, usage))
            .flatten()
    }

    /// A key message to send now, or `None` when it waits for a switch
    pub fn hold(&mut self, message: Vec<u8>) -> Option<Vec<u8>> {
        match &mut self.switching {
            Some(switching) => {
                switching.hold(message);
                None
            }
            None => Some(message),
        }
    }

    /// When keys stop waiting for a switch the Mac does not report
    pub fn deadline(&self) -> Option<Instant> {
        self.switching.as_ref().map(Switching::deadline)
    }

    /// The messages held for a switch, once the Mac reported it or stopped being waited for
    pub fn switched(&mut self) -> Vec<Vec<u8>> {
        self.switching.take().map(Switching::release).unwrap_or_default()
    }

    // Asks for the source of the client's language when the option is on and the Mac has reported its own.
    fn follow(&mut self, now: Instant) -> Vec<Vec<u8>> {
        if !self.syncs || self.reported.is_none() {
            return Vec::new();
        }

        let Some(id) = self.target().filter(|id| self.asked.as_ref() != Some(id)) else {
            return Vec::new();
        };

        let (switching, messages) = Switching::start(&id, self.switching.take(), now);
        self.switching = Some(switching);
        self.asked = Some(id);
        messages
    }

    // The Mac source that types the client's language
    fn target(&self) -> Option<String> {
        keyboard::source(self.language.as_deref()?, self.reported.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::Sources;
    use crate::message::{InputSource, client};
    use std::time::Instant;

    const KOREAN: &str = "com.apple.inputmethod.Korean.2SetKorean";
    const ABC: &str = "com.apple.keylayout.ABC";
    // The F key, which types ㄹ in Korean
    const F: u16 = 0x09;
    const RIEUL: u32 = 0x0100_3139;

    fn source(id: &str) -> InputSource {
        InputSource {
            id: id.into(),
            secure: false,
        }
    }

    // Nothing is asked before the Mac reports its source, which decides the mode of a language, and a source is asked
    // only once.
    #[test]
    fn asks_once_the_mac_reported_and_only_for_a_new_source() {
        let now = Instant::now();
        let mut sources = Sources::default();

        assert!(sources.sync(true, now).is_empty());
        assert!(sources.language("ko-KR".into(), now).is_empty());

        let messages = sources.reported(source(ABC), now);
        assert_eq!(messages[1], client::input_source(KOREAN));
        assert_eq!(
            sources.reported(source(KOREAN), now),
            [client::key(0xff_ffff, false).to_vec()]
        );
        assert!(sources.language("ko-KR".into(), now).is_empty());
    }

    #[test]
    fn turning_off_releases_only_a_source_asked_for() {
        let now = Instant::now();
        let mut sources = Sources::default();
        sources.language("ko-KR".into(), now);
        sources.reported(source(KOREAN), now);

        assert!(sources.sync(false, now).is_empty());

        sources.sync(true, now);
        sources.reported(source(KOREAN), now);
        assert_eq!(sources.sync(false, now), [client::input_source("")]);
    }

    // With the option on the source asked for counts as the Mac's; with it off, the one the Mac reported.
    #[test]
    fn keys_go_with_their_physical_key_only_on_the_source_of_the_language() {
        let now = Instant::now();
        let mut sources = Sources::default();
        sources.language("ko-KR".into(), now);
        sources.reported(source(ABC), now);
        assert_eq!(sources.keycode(RIEUL, Some(F)), None);

        sources.reported(source(KOREAN), now);
        assert_eq!(sources.keycode(RIEUL, Some(F)), Some(0x03));

        sources.sync(true, now);
        sources.reported(source(ABC), now);
        assert_eq!(sources.keycode(RIEUL, Some(F)), Some(0x03));
    }
}
