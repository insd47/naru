use crate::message::{InputSource, client};
use layout::Layout;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

mod layout;
mod sources;
mod tables;

// 확인을 기다리다 지정을 다시 보내기까지의 시간과, 확인 없이 지정을 보내는 횟수입니다.
// Mac은 지정을 곧바로 반영하지만 알림(`0x455`)은 늦게 보냈고, 빠르게 여러 번 바꾸면 순서가 뒤섞여 오기도 했습니다.
const CONFIRM: Duration = Duration::from_millis(1000);
const ATTEMPTS: u32 = 2;

const ENGLISH: &str = "com.apple.keylayout.ABC";

// Control, Option(Meta, Alt), Command(Super)의 왼쪽과 오른쪽. 이 키가 눌려 있으면 단축키이므로 글자를 바꾸지 않습니다.
const SHORTCUT_MODIFIERS: [u32; 8] = [0xffe3, 0xffe4, 0xffe7, 0xffe8, 0xffe9, 0xffea, 0xffeb, 0xffec];

/// 키 입력을 Mac의 입력 소스에 맞춰 보냅니다.
///
/// - 언어는 태블릿이 고르고, 글자는 언제나 고른 입력 소스의 배열대로 보냅니다. 지정과 키는 같은 순서로 도착하므로 Mac은 바뀐 입력 소스로 키를 받습니다.
/// - Mac이 다른 입력 소스를 알리면(늦게 온 알림, Mac 쪽 Caps Lock 전환) 고른 입력 소스를 다시 지정합니다. 태블릿에 보이는 언어와 실제로 들어가는 언어가 갈리지 않게 합니다.
/// - 확인 없이 정한 횟수를 넘기면 Mac이 받지 않는 것으로 보고, 확인될 때까지 글자는 Mac이 알린 입력 소스를 따릅니다.
/// - 글자는 2벌식처럼 라틴 문자를 칠 수 없는 배열이면 그 배열의 글자로 바꾸고, 조합은 Mac이 합니다. 단축키, 보안 입력, 아직 입력 소스를 모를 때는 바꾸지 않습니다.
/// - 보안 입력(비밀번호 칸) 중에는 Mac이 ABC를 강제하므로 지정하지 않고 기다렸다가, 끝나면 고른 언어를 다시 지정합니다.
/// - 뗄 때는 누를 때 보낸 keysym을 보냅니다. 그 사이 입력 소스가 바뀌어도 Mac에 눌린 키가 남지 않습니다.
pub struct Keyboard {
    // 고른 언어의 Mac 입력 소스
    selected: Option<&'static str>,
    mac: Option<InputSource>,
    // 글자를 바꿀 배열. 고른 입력 소스의 것이고, Mac이 받지 않으면 Mac이 알린 입력 소스의 것입니다.
    layout: Option<Layout>,
    // 확인을 기다리는 지정을 보낸 시각
    pending: Option<Instant>,
    // 확인 없이 지정을 보낸 횟수. 확인되면 처음부터 셉니다.
    attempts: u32,
    // 확인 없이 횟수를 넘겨 Mac이 고른 입력 소스를 받지 않는 것으로 본 상태
    refused: bool,
    // 누른 키와 그때 보낸 keysym
    pressed: HashMap<u32, u32>,
    modifiers: HashSet<u32>,
}

impl Keyboard {
    pub fn new() -> Self {
        Self {
            selected: None,
            mac: None,
            layout: None,
            pending: None,
            attempts: 0,
            refused: false,
            pressed: HashMap::new(),
            modifiers: HashSet::new(),
        }
    }

    /// 언어를 고릅니다. `language`는 BCP 47 태그(`ko-KR`, `en-US`)입니다. Mac에 지정할 메시지를 돌려줍니다.
    pub fn select(&mut self, language: &str, now: Instant) -> Option<Vec<u8>> {
        self.selected = Some(sources::source(language).unwrap_or(ENGLISH));
        self.attempts = 0;
        self.refused = false;
        let message = self.request(now);
        self.refresh();
        message
    }

    /// Mac이 알린 입력 소스를 반영합니다. 고른 입력 소스와 다르면 다시 지정하는 메시지를 돌려줍니다.
    ///
    /// - 확인을 기다리는 동안 온 다른 입력 소스는 지정 전의 상태이므로 다시 지정하지 않습니다.
    pub fn observe(&mut self, source: InputSource, now: Instant) -> Option<Vec<u8>> {
        let confirmed = self.selected == Some(source.id.as_str());
        let secure = source.secure;
        self.mac = Some(source);

        if confirmed {
            self.pending = None;
            self.attempts = 0;
            self.refused = false;
        }

        // 보안 입력 중에는 Mac이 지정을 받지 않으므로 기다림을 끝냅니다.
        if secure {
            self.pending = None;
        }

        let message = (!confirmed && !secure && self.pending.is_none() && !self.refused)
            .then(|| self.request(now))
            .flatten();
        self.refresh();
        message
    }

    /// 지정을 다시 보낼 시각. 확인을 기다리지 않으면 `None`입니다.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.map(|at| at + CONFIRM)
    }

    /// 확인이 오지 않았습니다. 횟수가 남았으면 다시 지정합니다.
    pub fn expire(&mut self, now: Instant) -> Option<Vec<u8>> {
        self.pending = None;
        let message = self.request(now);
        self.refresh();
        message
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

        // 보안 입력 중에는 알린 입력 소스와 관계없이 Mac이 ABC로 받습니다.
        let secure = self.mac.as_ref().is_some_and(|mac| mac.secure);
        let sent = match &self.layout {
            Some(layout) if self.modifiers.is_empty() && !secure => layout.translate(keysym),
            _ => keysym,
        };
        self.pressed.insert(keysym, sent);
        client::key(sent, true).to_vec()
    }

    // 고른 언어를 지정합니다. 보안 입력 중이면 Mac이 받지 않으므로 끝날 때 다시 지정합니다.
    // 다른 지정이 확인을 기다리는 중이면 Mac이 이미 그 언어로 바꿨을 수 있으므로, 알린 입력 소스와 같아도 지정합니다.
    // Mac이 아직 입력 소스를 알리지 않았으면 기다리지 않고 보냅니다. 알리면 고른 입력 소스와 비교해 다시 지정합니다.
    fn request(&mut self, now: Instant) -> Option<Vec<u8>> {
        let source = self.selected?;
        let message = InputSource::request(source);

        let Some(mac) = &self.mac else {
            return Some(message);
        };

        if mac.secure || (mac.id == source && self.pending.is_none()) {
            return None;
        }

        if self.attempts >= ATTEMPTS {
            self.refused = true;
            return None;
        }

        self.attempts += 1;
        self.pending = Some(now);
        Some(message)
    }

    // Mac이 아직 입력 소스를 알리지 않았으면 잠금 화면일 수 있으므로 글자를 바꾸지 않습니다.
    fn refresh(&mut self) {
        let source = match (&self.mac, self.selected) {
            (None, _) => None,
            (Some(_), Some(selected)) if !self.refused => Some(selected),
            (Some(mac), _) => Some(mac.id.as_str()),
        };
        self.layout = source.and_then(Layout::find);
    }
}

#[cfg(test)]
mod tests {
    use super::{CONFIRM, ENGLISH, Keyboard};
    use crate::message::{InputSource, client};
    use std::time::Instant;

    const KOREAN: &str = "com.apple.inputmethod.Korean.2SetKorean";
    const R: u32 = b'r' as u32;
    const GIYEOK: u32 = 0x0100_3131;
    const COMMAND: u32 = 0xffeb;

    fn source(id: &str, secure: bool) -> InputSource {
        InputSource { id: id.into(), secure }
    }

    fn key(keysym: u32, down: bool) -> Vec<u8> {
        client::key(keysym, down).to_vec()
    }

    fn request(id: &str) -> Option<Vec<u8>> {
        Some(InputSource::request(id))
    }

    fn english_mac() -> Keyboard {
        let mut keyboard = Keyboard::new();
        keyboard.observe(source(ENGLISH, false), Instant::now());
        keyboard
    }

    fn korean_mac() -> Keyboard {
        let mut keyboard = english_mac();
        let now = Instant::now();
        keyboard.select("ko-KR", now);
        keyboard.observe(source(KOREAN, false), now);
        keyboard
    }

    #[test]
    fn keys_right_after_switching_follow_the_new_source() {
        let mut keyboard = english_mac();

        assert_eq!(keyboard.select("ko-KR", Instant::now()), request(KOREAN));
        assert_eq!(keyboard.key(R, true), key(GIYEOK, true));
    }

    // Mac은 지정을 받기 전의 입력 소스를 알리기도 합니다. 확인 전에는 지정한 입력 소스를 따릅니다.
    #[test]
    fn notice_of_another_source_before_the_confirmation_is_ignored() {
        let mut keyboard = english_mac();
        let now = Instant::now();
        keyboard.select("ko-KR", now);

        assert_eq!(keyboard.observe(source(ENGLISH, false), now), None);
        assert_eq!(keyboard.key(R, true), key(GIYEOK, true));
    }

    #[test]
    fn unconfirmed_request_retries_once_then_follows_the_mac() {
        let mut keyboard = english_mac();
        let now = Instant::now();
        keyboard.select("ko-KR", now);

        assert_eq!(keyboard.deadline(), Some(now + CONFIRM));
        assert_eq!(keyboard.expire(now), request(KOREAN));
        assert_eq!(keyboard.expire(now), None);
        assert_eq!(keyboard.deadline(), None);
        assert_eq!(keyboard.key(R, true), key(R, true));
    }

    #[test]
    fn toggling_back_before_the_confirmation_still_requests() {
        let mut keyboard = english_mac();
        let now = Instant::now();
        keyboard.select("ko-KR", now);

        assert_eq!(keyboard.select("en-US", now), request(ENGLISH));
        assert_eq!(keyboard.observe(source(KOREAN, false), now), None);
        assert_eq!(keyboard.key(R, true), key(R, true));
        keyboard.observe(source(ENGLISH, false), now);
        assert_eq!(keyboard.deadline(), None);
    }

    #[test]
    fn shortcuts_stay_latin_in_korean() {
        let mut keyboard = korean_mac();
        keyboard.key(COMMAND, true);

        assert_eq!(keyboard.key(R, true), key(R, true));
        keyboard.key(R, false);
        keyboard.key(COMMAND, false);
        assert_eq!(keyboard.key(R, true), key(GIYEOK, true));
    }

    #[test]
    fn repeat_and_release_keep_the_keysym_that_was_pressed() {
        let mut keyboard = korean_mac();
        keyboard.key(R, true);
        keyboard.observe(source(ENGLISH, true), Instant::now());

        assert_eq!(keyboard.key(R, true), key(GIYEOK, true));
        assert_eq!(keyboard.key(R, false), key(GIYEOK, false));
    }

    #[test]
    fn secure_input_defers_the_request_until_it_ends() {
        let mut keyboard = Keyboard::new();
        let now = Instant::now();
        keyboard.observe(source(ENGLISH, true), now);

        assert_eq!(keyboard.select("ko-KR", now), None);
        assert_eq!(keyboard.key(R, true), key(R, true));
        assert_eq!(keyboard.observe(source(ENGLISH, false), now), request(KOREAN));
    }

    // 비밀번호 칸으로 들어갈 때 Mac은 보안 입력이 아닌 ABC를 먼저 알리고, 곧 보안 입력을 켰다 끕니다.
    #[test]
    fn secure_input_flicker_restores_the_selected_language() {
        let mut keyboard = korean_mac();
        let now = Instant::now();

        assert_eq!(keyboard.observe(source(ENGLISH, false), now), request(KOREAN));
        assert_eq!(keyboard.observe(source(ENGLISH, true), now), None);
        assert_eq!(keyboard.observe(source(ENGLISH, false), now), request(KOREAN));
    }

    // 접속 직후 잠금 화면의 비밀번호를 곧바로 칠 수 있어야 합니다.
    #[test]
    fn keys_go_out_as_latin_until_the_mac_reports_its_source() {
        let mut keyboard = Keyboard::new();
        let now = Instant::now();

        assert_eq!(keyboard.select("ko-KR", now), request(KOREAN));
        assert_eq!(keyboard.key(R, true), key(R, true));
        assert_eq!(keyboard.deadline(), None);
    }

    #[test]
    fn first_report_requests_the_selected_language_again() {
        let mut keyboard = Keyboard::new();
        let now = Instant::now();
        keyboard.select("ko-KR", now);

        assert_eq!(keyboard.observe(source(ENGLISH, false), now), request(KOREAN));
        assert_eq!(keyboard.key(R, true), key(GIYEOK, true));
    }

    #[test]
    fn selecting_the_current_source_sends_nothing() {
        let mut keyboard = korean_mac();
        assert_eq!(keyboard.select("ko-KR", Instant::now()), None);
        assert_eq!(keyboard.deadline(), None);
    }

    #[test]
    fn shifted_double_consonant_maps_separately() {
        let mut keyboard = korean_mac();
        assert_eq!(keyboard.key(b'R' as u32, true), key(0x0100_3132, true));
        assert_eq!(keyboard.key(b'A' as u32, true), key(0x0100_3141, true));
    }

    // Mac은 비밀번호 칸에 들어가며 한국어 입력 소스를 보안 입력과 함께 알리기도 합니다. 이때 자모를 보내면 낱자로 들어갑니다.
    #[test]
    fn secure_input_sends_latin_even_while_the_source_reads_korean() {
        let mut keyboard = korean_mac();
        keyboard.observe(source(KOREAN, true), Instant::now());

        assert_eq!(keyboard.key(R, true), key(R, true));
    }

    #[test]
    fn languages_without_a_source_type_in_english() {
        let mut keyboard = korean_mac();
        assert_eq!(keyboard.select("de-DE", Instant::now()), request(ENGLISH));
    }

    #[test]
    fn latin_script_tags_type_in_english() {
        let mut keyboard = english_mac();
        assert_eq!(keyboard.select("sr-Latn-RS", Instant::now()), None);
        assert_eq!(
            keyboard.select("sr-RS", Instant::now()),
            request("com.apple.keylayout.Serbian")
        );
    }

    // Mac 쪽 Caps Lock 전환이나 늦게 온 알림으로 Mac이 다른 입력 소스를 알려도 태블릿이 고른 언어를 지킵니다.
    #[test]
    fn mac_side_change_is_reverted_to_the_selected_language() {
        let mut keyboard = korean_mac();

        assert_eq!(
            keyboard.observe(source(ENGLISH, false), Instant::now()),
            request(KOREAN)
        );
        assert_eq!(keyboard.key(R, true), key(GIYEOK, true));
    }

    #[test]
    fn refused_source_follows_the_mac_until_confirmed() {
        let mut keyboard = korean_mac();
        let now = Instant::now();
        keyboard.observe(source(ENGLISH, false), now);

        assert_eq!(keyboard.expire(now), request(KOREAN));
        assert_eq!(keyboard.expire(now), None);
        assert_eq!(keyboard.observe(source(ENGLISH, false), now), None);
        assert_eq!(keyboard.key(R, true), key(R, true));
        keyboard.key(R, false);

        keyboard.observe(source(KOREAN, false), now);
        assert_eq!(keyboard.key(R, true), key(GIYEOK, true));
    }
}
