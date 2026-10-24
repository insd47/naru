//! What the Mac's keyboard layouts type, and which Mac input source types a language, as pure functions for clients.
//!
//! - The Mac stays on the user's native-language input method, and the client decides per key what to type. The agent
//!   looks up each received character in the Mac's current layout and presses that key, so a Korean Mac must receive
//!   `ㄱ`, not `r`, for its input method to compose. A key sent with its physical key code is pressed as that key.
//! - Nothing here holds session state. Clients pass the Mac's latest input source from `Event::Source`.
use crate::message::InputSource;
use layout::Layout;

mod keycodes;
mod languages;
mod layout;
mod tables;

pub use keycodes::keycode;

const ABC: &str = "com.apple.keylayout.ABC";

/// What to send so the Mac types what this Latin key means in `language`, given the Mac's current input source.
///
/// - `latin` is the keysym of the Latin letter, digit, or symbol on the physical key, shifted as typed. `language` is
///   the client's input language as a BCP 47 tag (`ko-KR`).
/// - Returns `None` when the language has no Mac layout to follow (Latin-script languages, Japanese Romaji, Chinese
///   Pinyin): send what the client's keyboard produced.
/// - In secure input, such as a password field, the Mac forces ABC, so the Latin key itself is returned.
/// - When the Mac is on another layout of the same language (Sebeolsik, Kana typing), that layout's character is
///   returned.
/// - Shortcuts are not characters: with Command, Control, or Option held, send the Latin key without calling this.
pub fn character(latin: u32, language: &str, source: Option<&InputSource>) -> Option<u32> {
    if source.is_some_and(|source| source.secure) {
        return Some(latin);
    }

    let mac = source.map(|source| source.id.as_str());
    let layout = languages::layout(language, mac).and_then(Layout::find)?;
    Some(layout.translate(latin))
}

/// The Mac input source to select so the Mac follows the client's input language, as Apple's "Sync Keyboard Language"
/// does. `None` in secure input, where the Mac forces ABC.
///
/// - A Mac already on another mode of the same language (Sebeolsik, Kana typing) keeps it. Latin languages get ABC.
/// - With more than one non-Latin language the Mac may show the new source but keep typing with the old one, as with
///   Apple's client.
pub fn source(language: &str, current: Option<&InputSource>) -> Option<String> {
    if current.is_some_and(|current| current.secure) {
        return None;
    }

    Some(languages::source(language, current.map(|current| current.id.as_str())))
}

#[cfg(test)]
mod tests {
    use super::character;
    use crate::message::InputSource;

    const R: u32 = b'r' as u32;
    const GIYEOK: u32 = 0x0100_3131;

    fn source(id: &str, secure: bool) -> InputSource {
        InputSource { id: id.into(), secure }
    }

    #[test]
    fn korean_types_jamo_and_english_has_no_layout() {
        assert_eq!(character(R, "ko-KR", None), Some(GIYEOK));
        assert_eq!(character(R, "en-US", None), None);
    }

    #[test]
    fn shifted_double_consonant_maps_separately() {
        assert_eq!(character(b'R' as u32, "ko-KR", None), Some(0x0100_3132));
        assert_eq!(character(b'A' as u32, "ko-KR", None), Some(0x0100_3141));
    }

    // In a password field the Mac forces ABC, so sending jamo would enter them as separate letters.
    #[test]
    fn secure_input_types_latin() {
        let korean = source("com.apple.inputmethod.Korean.2SetKorean", true);
        assert_eq!(character(R, "ko-KR", Some(&korean)), Some(R));
    }

    // With a Kana typing Mac, Japanese gets the Kana layout character at that position, and English stays Latin.
    #[test]
    fn kana_typing_mac_receives_kana() {
        let kana = source("com.apple.inputmethod.Kotoeri.KanaTyping.Japanese", false);
        assert_eq!(character(b'q' as u32, "ja-JP", Some(&kana)), Some(0x0100_305F));
        assert_eq!(character(b'q' as u32, "en-US", Some(&kana)), None);
    }

    #[test]
    fn sebeolsik_mac_receives_its_own_layout() {
        let three_set = source("com.apple.inputmethod.Korean.3SetKorean", false);
        let two_set = character(R, "ko-KR", None);
        assert!(character(R, "ko-KR", Some(&three_set)).is_some_and(|keysym| Some(keysym) != two_set));
    }
}
