use crate::message::{InputSource, client};
use layout::Layout;
use std::collections::{HashMap, HashSet};

mod languages;
mod layout;
mod tables;

// Left and right Control, Option (Meta, Alt), and Command (Super). While one is held the key is a shortcut, so characters are not converted.
const SHORTCUT_MODIFIERS: [u32; 8] = [0xffe3, 0xffe4, 0xffe7, 0xffe8, 0xffe9, 0xffea, 0xffeb, 0xffec];

/// Sends key input according to the tablet's input language.
///
/// - The agent looks up each received character in the Mac's current layout and enters its key code, or enters the character as is when not found.
///   So the Mac stays on the user's native-language input method and the language is switched on the tablet. With Dubeolsik, jamo become keys that the Mac input method composes,
///   while English Latin letters are absent from the Dubeolsik layout and go in as is.
/// - Characters are converted to the character at the same position in the Mac layout for the tablet language. If the Mac is on another layout of that language (Sebeolsik, Kana typing), that layout is used.
///   Nothing is converted for shortcuts or during secure input.
/// - The Mac's input source is never set. Switching it with `0x1a` changed only the input source, while the input method kept the previous layout.
/// - A release sends the keysym sent on press, so no key stays held on the Mac when the language changes in between.
pub struct Keyboard {
    // Tablet language as a BCP 47 tag
    language: Option<String>,
    // Input source reported by the Mac. If it is a mode of the same language, its layout is used.
    mac: Option<InputSource>,
    // Layout to convert characters with, absent for languages that send Latin letters as is
    layout: Option<Layout>,
    // Pressed keys and the keysyms sent for them
    pressed: HashMap<u32, u32>,
    modifiers: HashSet<u32>,
}

impl Keyboard {
    pub fn new() -> Self {
        Self {
            language: None,
            mac: None,
            layout: None,
            pressed: HashMap::new(),
            modifiers: HashSet::new(),
        }
    }

    /// Changes the tablet language. `language` is a BCP 47 tag (`ko-KR`, `en-US`).
    pub fn select(&mut self, language: &str) {
        self.language = Some(language.to_owned());
        self.refresh();
    }

    /// Applies the input source the Mac reported, which sets secure input and the layout to follow for the same language.
    pub fn observe(&mut self, source: &InputSource) {
        let changed = self.mac.as_ref().is_none_or(|mac| mac.id != source.id);
        self.mac = Some(source.clone());

        if changed {
            self.refresh();
        }
    }

    /// The message that presses or releases a key
    pub fn key(&mut self, keysym: u32, down: bool) -> Vec<u8> {
        if !down {
            self.modifiers.remove(&keysym);
            let sent = self.pressed.remove(&keysym).unwrap_or(keysym);
            return client::key(sent, false).to_vec();
        }

        // Auto-repeat presses reuse the keysym sent first.
        if let Some(&sent) = self.pressed.get(&keysym) {
            return client::key(sent, true).to_vec();
        }

        if SHORTCUT_MODIFIERS.contains(&keysym) {
            self.modifiers.insert(keysym);
        }

        // During secure input (a password field) the Mac forces ABC.
        let secure = self.mac.as_ref().is_some_and(|mac| mac.secure);
        let sent = match &self.layout {
            Some(layout) if self.modifiers.is_empty() && !secure => layout.translate(keysym),
            _ => keysym,
        };
        self.pressed.insert(keysym, sent);
        client::key(sent, true).to_vec()
    }

    fn refresh(&mut self) {
        let mac = self.mac.as_ref().map(|mac| mac.id.as_str());
        self.layout = self
            .language
            .as_deref()
            .and_then(|language| languages::layout(language, mac))
            .and_then(Layout::find);
    }
}

#[cfg(test)]
mod tests {
    use super::Keyboard;
    use crate::message::{InputSource, client};

    const R: u32 = b'r' as u32;
    const GIYEOK: u32 = 0x0100_3131;
    const COMMAND: u32 = 0xffeb;

    fn key(keysym: u32, down: bool) -> Vec<u8> {
        client::key(keysym, down).to_vec()
    }

    fn korean() -> Keyboard {
        let mut keyboard = Keyboard::new();
        keyboard.select("ko-KR");
        keyboard
    }

    fn source(id: &str, secure: bool) -> InputSource {
        InputSource { id: id.into(), secure }
    }

    fn secure(secure: bool) -> InputSource {
        source("com.apple.inputmethod.Korean.2SetKorean", secure)
    }

    #[test]
    fn korean_sends_jamo_and_english_sends_latin() {
        let mut keyboard = korean();
        assert_eq!(keyboard.key(R, true), key(GIYEOK, true));
        keyboard.key(R, false);

        keyboard.select("en-US");
        assert_eq!(keyboard.key(R, true), key(R, true));
    }

    #[test]
    fn shortcuts_stay_latin_in_korean() {
        let mut keyboard = korean();
        keyboard.key(COMMAND, true);

        assert_eq!(keyboard.key(R, true), key(R, true));
        keyboard.key(R, false);
        keyboard.key(COMMAND, false);
        assert_eq!(keyboard.key(R, true), key(GIYEOK, true));
    }

    #[test]
    fn repeat_and_release_keep_the_keysym_that_was_pressed() {
        let mut keyboard = korean();
        keyboard.key(R, true);
        keyboard.select("en-US");

        assert_eq!(keyboard.key(R, true), key(GIYEOK, true));
        assert_eq!(keyboard.key(R, false), key(GIYEOK, false));
    }

    // In a password field the Mac forces ABC, so sending jamo would enter them as separate letters.
    #[test]
    fn secure_input_sends_latin() {
        let mut keyboard = korean();
        keyboard.observe(&secure(true));
        assert_eq!(keyboard.key(R, true), key(R, true));
        keyboard.key(R, false);

        keyboard.observe(&secure(false));
        assert_eq!(keyboard.key(R, true), key(GIYEOK, true));
    }

    #[test]
    fn shifted_double_consonant_maps_separately() {
        let mut keyboard = korean();
        assert_eq!(keyboard.key(b'R' as u32, true), key(0x0100_3132, true));
        assert_eq!(keyboard.key(b'A' as u32, true), key(0x0100_3141, true));
    }

    #[test]
    fn languages_without_a_table_send_latin() {
        let mut keyboard = Keyboard::new();
        keyboard.select("ja-JP");
        assert_eq!(keyboard.key(R, true), key(R, true));
    }

    // With a Kana typing Mac, Japanese sends the Kana layout character at that position so it maps to a key, and English goes as is.
    #[test]
    fn kana_typing_mac_receives_kana() {
        let mut keyboard = Keyboard::new();
        keyboard.observe(&source("com.apple.inputmethod.Kotoeri.KanaTyping.Japanese", false));
        keyboard.select("ja-JP");
        assert_eq!(keyboard.key(b'q' as u32, true), key(0x0100_305F, true));
        keyboard.key(b'q' as u32, false);

        keyboard.select("en-US");
        assert_eq!(keyboard.key(b'q' as u32, true), key(b'q' as u32, true));
    }
}
