use crate::message::{InputSource, client};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

// 지정한 입력 소스를 Mac이 확인하기를 기다리는 시간과 지정을 다시 보내는 횟수입니다.
// 기다리는 동안 친 키는 붙잡아 두므로 짧게 둡니다. 시간이 지나면 붙잡아 둔 키를 Mac의 지금 입력 소스대로 보냅니다.
const CONFIRM: Duration = Duration::from_millis(400);
const ATTEMPTS: u32 = 3;

const KOREAN: &str = "com.apple.inputmethod.Korean.2SetKorean";
const ENGLISH: &str = "com.apple.keylayout.ABC";

// Control, Option(Meta, Alt), Command(Super)의 왼쪽과 오른쪽. 이 키가 눌려 있으면 단축키이므로 자모로 바꾸지 않습니다.
const SHORTCUT_MODIFIERS: [u32; 8] = [0xffe3, 0xffe4, 0xffe7, 0xffe8, 0xffe9, 0xffea, 0xffeb, 0xffec];

// 2벌식 자판의 라틴 글자와 호환 자모. Shift가 뜻을 바꾸는 쌍자음과 ㅒ, ㅖ만 대문자를 따로 둡니다.
const LOWER: &str = "qwertyuiopasdfghjklzxcvbnm";
const JAMO: &str = "ㅂㅈㄷㄱㅅㅛㅕㅑㅐㅔㅁㄴㅇㄹㅎㅗㅓㅏㅣㅋㅌㅊㅍㅠㅜㅡ";
const SHIFTED: &str = "QWERTOP";
const SHIFTED_JAMO: &str = "ㅃㅉㄸㄲㅆㅒㅖ";

/// 사용자가 고른 키보드 언어입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Language {
    /// ABC 자판
    English,
    /// 2벌식
    Korean,
}

impl Language {
    fn source(self) -> &'static str {
        match self {
            Self::English => ENGLISH,
            Self::Korean => KOREAN,
        }
    }
}

/// 키 입력을 Mac의 입력 소스에 맞춰 보냅니다.
///
/// - 언어는 클라이언트가 고릅니다. 고르면 Mac에 입력 소스를 지정하고, 확인(`0x455`)이 올 때까지 친 키는 붙잡아 두었다가 확인되면 보냅니다.
/// - 글자는 Mac의 지금 입력 소스대로 바꿉니다. 2벌식이면 호환 자모로 보내고 조합은 Mac이 합니다. 단축키와 아직 입력 소스를 모를 때는 바꾸지 않습니다.
/// - 보안 입력(비밀번호 칸) 중에는 Mac이 ABC를 강제하므로 지정하지 않고 기다렸다가, 끝나면 고른 언어를 다시 지정합니다.
/// - Mac이 스스로 바꾸면(Mac 쪽 Caps Lock 전환 등) 글자는 그 입력 소스를 따르고, 고른 언어는 그대로 둡니다. 다음에 고를 때 다시 지정합니다.
/// - 뗄 때는 누를 때 보낸 keysym을 보냅니다. 그 사이 입력 소스가 바뀌어도 Mac에 눌린 키가 남지 않습니다.
pub struct Keyboard {
    language: Option<Language>,
    mac: Option<InputSource>,
    pending: Option<Pending>,
    // 지금 고른 언어로 확인 없이 지정을 보낸 횟수
    attempts: u32,
    held: VecDeque<(u32, bool)>,
    // 누른 키와 그때 보낸 keysym
    pressed: HashMap<u32, u32>,
    modifiers: HashSet<u32>,
    jamo: HashMap<u32, u32>,
}

// 확인을 기다리는 지정
struct Pending {
    language: Language,
    at: Instant,
}

impl Keyboard {
    pub fn new() -> Self {
        let lower = LOWER.chars().zip(JAMO.chars());
        let upper = LOWER.chars().map(|latin| latin.to_ascii_uppercase()).zip(JAMO.chars());
        let shifted = SHIFTED.chars().zip(SHIFTED_JAMO.chars());
        let jamo = lower
            .chain(upper)
            .chain(shifted)
            .map(|(latin, jamo)| (u32::from(latin), unicode(jamo)))
            .collect();

        Self {
            language: None,
            mac: None,
            pending: None,
            attempts: 0,
            held: VecDeque::new(),
            pressed: HashMap::new(),
            modifiers: HashSet::new(),
            jamo,
        }
    }

    /// 언어를 고릅니다. Mac에 지정할 메시지를 돌려줍니다.
    pub fn select(&mut self, language: Language, now: Instant) -> Vec<Vec<u8>> {
        self.language = Some(language);
        self.attempts = 0;
        self.request(now)
    }

    /// 키를 누르거나 뗍니다. 확인을 기다리는 동안에는 붙잡아 두고 빈 목록을 돌려줍니다.
    ///
    /// - 이미 보낸 키를 떼는 것은 붙잡지 않습니다. 같은 키의 이벤트가 붙잡혀 있으면 순서를 지키려고 함께 붙잡습니다.
    pub fn key(&mut self, keysym: u32, down: bool) -> Vec<Vec<u8>> {
        let sent = !down && self.pressed.contains_key(&keysym) && self.held.iter().all(|&(held, _)| held != keysym);

        if self.pending.is_some() && !sent {
            self.held.push_back((keysym, down));
            return Vec::new();
        }

        vec![self.translate(keysym, down)]
    }

    /// Mac이 알린 입력 소스를 반영합니다. 확인이면 붙잡아 둔 키를, 보안 입력이 끝났으면 다시 지정하는 메시지를 돌려줍니다.
    pub fn observe(&mut self, source: InputSource, now: Instant) -> Vec<Vec<u8>> {
        let was_secure = self.mac.as_ref().is_some_and(|mac| mac.secure);
        let confirmed = self
            .pending
            .as_ref()
            .is_some_and(|pending| source.id == pending.language.source());
        let secure = source.secure;
        self.mac = Some(source);

        if secure || confirmed {
            return self.release();
        }

        if was_secure && self.pending.is_none() {
            self.attempts = 0;
            return self.request(now);
        }

        Vec::new()
    }

    /// 확인을 기다리는 시한. 기다리지 않으면 `None`입니다.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|pending| pending.at + CONFIRM)
    }

    /// 확인이 오지 않았습니다. 붙잡아 둔 키를 Mac의 지금 입력 소스대로 보내고, 횟수가 남았으면 다시 지정합니다.
    pub fn expire(&mut self, now: Instant) -> Vec<Vec<u8>> {
        let mut messages = self.release();

        if self.attempts < ATTEMPTS {
            messages.extend(self.request(now));
        }

        messages
    }

    // 고른 언어를 지정합니다. 보안 입력 중이면 Mac이 받지 않으므로 끝날 때 다시 지정합니다.
    // 다른 지정이 확인을 기다리는 중이면 Mac이 곧 그 언어로 바꿀 수 있으므로, 지금 입력 소스와 같아도 지정합니다.
    fn request(&mut self, now: Instant) -> Vec<Vec<u8>> {
        let Some(language) = self.language else {
            return Vec::new();
        };

        match &self.mac {
            Some(mac) if mac.secure => self.release(),
            Some(mac) if mac.id == language.source() && self.pending.is_none() => Vec::new(),
            _ => {
                self.attempts += 1;
                self.pending = Some(Pending { language, at: now });
                vec![InputSource::request(language.source())]
            }
        }
    }

    // 기다림을 끝내고 붙잡아 둔 키를 차례로 보냅니다.
    fn release(&mut self) -> Vec<Vec<u8>> {
        self.pending = None;
        let held = std::mem::take(&mut self.held);
        held.into_iter()
            .map(|(keysym, down)| self.translate(keysym, down))
            .collect()
    }

    fn translate(&mut self, keysym: u32, down: bool) -> Vec<u8> {
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

        let korean = self.mac.as_ref().is_some_and(|mac| mac.id == KOREAN);
        let jamo = (korean && self.modifiers.is_empty())
            .then(|| self.jamo.get(&keysym))
            .flatten();
        let sent = jamo.copied().unwrap_or(keysym);
        self.pressed.insert(keysym, sent);
        client::key(sent, true).to_vec()
    }
}

// 라틴-1 밖의 문자는 유니코드 keysym입니다.
fn unicode(char: char) -> u32 {
    0x0100_0000 + u32::from(char)
}

#[cfg(test)]
mod tests {
    use super::{ATTEMPTS, CONFIRM, ENGLISH, KOREAN, Keyboard, Language};
    use crate::message::{InputSource, client};
    use std::time::Instant;

    const R: u32 = b'r' as u32;
    const GIYEOK: u32 = 0x0100_3131;
    const COMMAND: u32 = 0xffeb;

    fn source(id: &str, secure: bool) -> InputSource {
        InputSource { id: id.into(), secure }
    }

    fn key(keysym: u32, down: bool) -> Vec<u8> {
        client::key(keysym, down).to_vec()
    }

    fn korean_mac() -> Keyboard {
        let mut keyboard = Keyboard::new();
        let now = Instant::now();
        keyboard.observe(source(ENGLISH, false), now);
        keyboard.select(Language::Korean, now);
        keyboard.observe(source(KOREAN, false), now);
        keyboard
    }

    #[test]
    fn keys_typed_before_the_confirmation_wait_and_follow_the_new_source() {
        let mut keyboard = Keyboard::new();
        let now = Instant::now();
        keyboard.observe(source(ENGLISH, false), now);

        assert_eq!(
            keyboard.select(Language::Korean, now),
            vec![InputSource::request(KOREAN)]
        );
        assert!(keyboard.key(R, true).is_empty());
        assert!(keyboard.key(R, false).is_empty());
        assert_eq!(
            keyboard.observe(source(KOREAN, false), now),
            vec![key(GIYEOK, true), key(GIYEOK, false)]
        );
    }

    #[test]
    fn unconfirmed_request_releases_keys_and_retries_a_bounded_number_of_times() {
        let mut keyboard = Keyboard::new();
        let now = Instant::now();
        keyboard.observe(source(ENGLISH, false), now);
        keyboard.select(Language::Korean, now);
        keyboard.key(R, true);

        assert_eq!(keyboard.deadline(), Some(now + CONFIRM));
        assert_eq!(keyboard.expire(now), vec![key(R, true), InputSource::request(KOREAN)]);

        for _ in 2..ATTEMPTS {
            assert_eq!(keyboard.expire(now), vec![InputSource::request(KOREAN)]);
        }

        assert!(keyboard.expire(now).is_empty());
        assert_eq!(keyboard.deadline(), None);
    }

    #[test]
    fn toggling_back_before_the_confirmation_still_requests() {
        let mut keyboard = Keyboard::new();
        let now = Instant::now();
        keyboard.observe(source(ENGLISH, false), now);
        keyboard.select(Language::Korean, now);

        assert_eq!(
            keyboard.select(Language::English, now),
            vec![InputSource::request(ENGLISH)]
        );
        assert!(keyboard.observe(source(KOREAN, false), now).is_empty());
        keyboard.observe(source(ENGLISH, false), now);
        assert_eq!(keyboard.deadline(), None);
    }

    #[test]
    fn shortcuts_stay_latin_in_korean() {
        let mut keyboard = korean_mac();
        keyboard.key(COMMAND, true);

        assert_eq!(keyboard.key(R, true), vec![key(R, true)]);
        keyboard.key(R, false);
        keyboard.key(COMMAND, false);
        assert_eq!(keyboard.key(R, true), vec![key(GIYEOK, true)]);
    }

    #[test]
    fn repeat_and_release_keep_the_keysym_that_was_pressed() {
        let mut keyboard = korean_mac();
        keyboard.key(R, true);
        keyboard.observe(source(ENGLISH, true), Instant::now());

        assert_eq!(keyboard.key(R, true), vec![key(GIYEOK, true)]);
        assert_eq!(keyboard.key(R, false), vec![key(GIYEOK, false)]);
    }

    #[test]
    fn releasing_a_key_sent_before_the_request_is_not_held() {
        let mut keyboard = Keyboard::new();
        let now = Instant::now();
        keyboard.observe(source(ENGLISH, false), now);
        keyboard.key(R, true);
        keyboard.select(Language::Korean, now);

        assert_eq!(keyboard.key(R, false), vec![key(R, false)]);
    }

    #[test]
    fn secure_input_defers_the_request_until_it_ends() {
        let mut keyboard = Keyboard::new();
        let now = Instant::now();
        keyboard.observe(source(ENGLISH, true), now);

        assert!(keyboard.select(Language::Korean, now).is_empty());
        assert_eq!(keyboard.key(R, true), vec![key(R, true)]);
        assert_eq!(
            keyboard.observe(source(ENGLISH, false), now),
            vec![InputSource::request(KOREAN)]
        );
    }

    // 비밀번호 칸으로 들어갈 때 Mac은 보안 입력이 아닌 ABC를 먼저 알리고, 곧 보안 입력을 켰다 끕니다.
    #[test]
    fn secure_input_flicker_restores_the_selected_language() {
        let mut keyboard = korean_mac();
        let now = Instant::now();

        assert!(keyboard.observe(source(ENGLISH, false), now).is_empty());
        assert!(keyboard.observe(source(ENGLISH, true), now).is_empty());
        assert_eq!(
            keyboard.observe(source(ENGLISH, false), now),
            vec![InputSource::request(KOREAN)]
        );
    }

    #[test]
    fn letters_stay_latin_until_the_mac_reports_its_source() {
        let mut keyboard = Keyboard::new();
        let now = Instant::now();
        keyboard.select(Language::Korean, now);
        keyboard.key(R, true);

        assert_eq!(keyboard.expire(now)[0], key(R, true));
    }

    #[test]
    fn selecting_the_current_source_sends_nothing() {
        let mut keyboard = korean_mac();
        assert!(keyboard.select(Language::Korean, Instant::now()).is_empty());
        assert_eq!(keyboard.deadline(), None);
    }

    #[test]
    fn shifted_double_consonant_maps_separately() {
        let mut keyboard = korean_mac();
        assert_eq!(keyboard.key(b'R' as u32, true), vec![key(0x0100_3132, true)]);
        assert_eq!(keyboard.key(b'A' as u32, true), vec![key(0x0100_3141, true)]);
    }
}
