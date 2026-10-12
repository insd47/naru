use crate::message::{InputSource, client};
use layout::Layout;
use std::collections::{HashMap, HashSet};

mod layout;
mod sources;
mod tables;

// Control, Option(Meta, Alt), Command(Super)의 왼쪽과 오른쪽. 이 키가 눌려 있으면 단축키이므로 글자를 바꾸지 않습니다.
const SHORTCUT_MODIFIERS: [u32; 8] = [0xffe3, 0xffe4, 0xffe7, 0xffe8, 0xffe9, 0xffea, 0xffeb, 0xffec];

/// 키 입력을 태블릿의 입력 언어에 맞춰 보냅니다.
///
/// - 에이전트는 받은 글자를 Mac의 지금 배열에서 찾아 키 코드로 넣고, 찾지 못하면 글자를 그대로 넣습니다.
///   그래서 Mac은 사용자의 모국어 입력기에 두고, 언어는 태블릿에서 바꿉니다. 2벌식이면 자모가 키가 되어 Mac 입력기가 조합하고,
///   영어의 라틴 문자는 2벌식 배열에 없어 그대로 들어갑니다.
/// - 글자는 태블릿 언어에 대응하는 Mac 배열의 같은 자리 글자로 바꿉니다. 단축키와 보안 입력 중에는 바꾸지 않습니다.
/// - Mac의 입력 소스는 지정하지 않습니다. `0x1a`로 바꾸면 입력 소스만 바뀌고 입력기가 쓰는 배열은 이전 것에 남았습니다.
/// - 뗄 때는 누를 때 보낸 keysym을 보냅니다. 그 사이 언어가 바뀌어도 Mac에 눌린 키가 남지 않습니다.
pub struct Keyboard {
    // 태블릿 언어에 대응하는 배열. 라틴 문자를 그대로 보내는 언어면 없습니다.
    layout: Option<Layout>,
    // 보안 입력(비밀번호 칸) 중이면 Mac이 ABC를 강제합니다.
    secure: bool,
    // 누른 키와 그때 보낸 keysym
    pressed: HashMap<u32, u32>,
    modifiers: HashSet<u32>,
}

impl Keyboard {
    pub fn new() -> Self {
        Self {
            layout: None,
            secure: false,
            pressed: HashMap::new(),
            modifiers: HashSet::new(),
        }
    }

    /// 태블릿 언어를 바꿉니다. `language`는 BCP 47 태그(`ko-KR`, `en-US`)입니다.
    pub fn select(&mut self, language: &str) {
        self.layout = sources::source(language).and_then(Layout::find);
    }

    /// Mac이 알린 입력 소스에서 보안 입력 여부를 반영합니다.
    pub fn observe(&mut self, source: &InputSource) {
        self.secure = source.secure;
    }

    /// 키를 누르거나 뗄 메시지
    pub fn key(&mut self, keysym: u32, down: bool) -> Vec<u8> {
        if !down {
            self.modifiers.remove(&keysym);
            let sent = self.pressed.remove(&keysym).unwrap_or(keysym);
            return client::key(sent, false).to_vec();
        }

        // 길게 눌러 반복되는 누름은 처음 보낸 keysym을 그대로 씁니다.
        if let Some(&sent) = self.pressed.get(&keysym) {
            return client::key(sent, true).to_vec();
        }

        if SHORTCUT_MODIFIERS.contains(&keysym) {
            self.modifiers.insert(keysym);
        }

        let sent = match &self.layout {
            Some(layout) if self.modifiers.is_empty() && !self.secure => layout.translate(keysym),
            _ => keysym,
        };
        self.pressed.insert(keysym, sent);
        client::key(sent, true).to_vec()
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

    fn secure(secure: bool) -> InputSource {
        InputSource {
            id: "com.apple.inputmethod.Korean.2SetKorean".into(),
            secure,
        }
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

    // 비밀번호 칸에서는 Mac이 ABC를 강제하므로 자모를 보내면 낱자로 들어갑니다.
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
}
