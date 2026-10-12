use crate::message::{InputSource, client};
use layout::Layout;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

mod layout;
mod sources;
mod tables;

// 확인을 기다리다 지정을 다시 보내기까지의 시간과, 확인 없이 지정을 보내는 횟수입니다.
// 에이전트는 받은 글자를 자기가 아는 지금 배열로 키에 되돌리는데, 지정한 뒤 그 배열이 바뀌어 알림(`0x455`)을 보내기까지는
// 다음 지정을 받아야 했습니다. 다시 보낸 지정의 약 30ms 뒤에 확인이 왔고, 곧바로 두 번 보낸 것은 소용이 없었습니다.
const RETRY: Duration = Duration::from_millis(150);
const ATTEMPTS: u32 = 7;

const ENGLISH: &str = "com.apple.keylayout.ABC";

// Control, Option(Meta, Alt), Command(Super)의 왼쪽과 오른쪽. 이 키가 눌려 있으면 단축키이므로 글자를 바꾸지 않습니다.
const SHORTCUT_MODIFIERS: [u32; 8] = [0xffe3, 0xffe4, 0xffe7, 0xffe8, 0xffe9, 0xffea, 0xffeb, 0xffec];

/// 키 입력을 Mac의 입력 소스에 맞춰 보냅니다.
///
/// - 언어는 태블릿이 고릅니다. 고르면 Mac에 입력 소스를 지정하고, 확인이 올 때까지 친 키는 붙잡아 두었다가 확인되면 보냅니다.
///   에이전트가 새 배열로 바뀌기 전에 보낸 글자는 키로 되돌아가지 못해 낱자로 들어가기 때문입니다.
/// - 글자는 에이전트가 알린 입력 소스의 배열대로 바꿉니다. 2벌식처럼 라틴 문자를 칠 수 없는 배열이면 그 배열의 글자로 바꾸고, 조합은 Mac이 합니다.
///   단축키, 보안 입력, 아직 입력 소스를 모를 때는 바꾸지 않습니다.
/// - 확인 없이 정한 횟수를 넘기면 붙잡아 둔 키를 Mac이 알린 입력 소스대로 보냅니다.
/// - Mac에서 바꾼 입력 소스(Ctrl+Space 등)는 따르고 다시 지정하지 않습니다. 에이전트가 예전 상태를 늦게 다시 알리기도 하므로, 알림마다 지정하면 서로 다툽니다.
/// - 보안 입력(비밀번호 칸) 중에는 Mac이 ABC를 강제하므로 지정하지 않고 기다렸다가, 끝나면 고른 언어를 다시 지정합니다.
/// - 뗄 때는 누를 때 보낸 keysym을 보냅니다. 그 사이 입력 소스가 바뀌어도 Mac에 눌린 키가 남지 않습니다.
pub struct Keyboard {
    // 고른 언어의 Mac 입력 소스
    selected: Option<&'static str>,
    mac: Option<InputSource>,
    // Mac이 알린 입력 소스의 배열
    layout: Option<Layout>,
    // 확인을 기다리는 지정을 마지막으로 보낸 시각
    pending: Option<Instant>,
    // 지금 고른 언어로 확인 없이 지정을 보낸 횟수
    attempts: u32,
    held: VecDeque<(u32, bool)>,
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
            held: VecDeque::new(),
            pressed: HashMap::new(),
            modifiers: HashSet::new(),
        }
    }

    /// 언어를 고릅니다. `language`는 BCP 47 태그(`ko-KR`, `en-US`)입니다. Mac에 지정할 메시지를 돌려줍니다.
    pub fn select(&mut self, language: &str, now: Instant) -> Vec<Vec<u8>> {
        self.selected = Some(sources::source(language).unwrap_or(ENGLISH));
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

    /// Mac이 알린 입력 소스를 반영합니다. 확인이면 붙잡아 둔 키를, 처음 알렸거나 보안 입력이 끝났으면 다시 지정하는 메시지를 돌려줍니다.
    ///
    /// - 확인을 기다리는 동안 온 다른 입력 소스는 지정 전의 상태이므로 붙잡아 둔 키를 그대로 둡니다.
    pub fn observe(&mut self, source: InputSource, now: Instant) -> Vec<Vec<u8>> {
        let first = self.mac.is_none();
        let was_secure = self.mac.as_ref().is_some_and(|mac| mac.secure);
        let confirmed = self.pending.is_some() && self.selected == Some(source.id.as_str());
        let secure = source.secure;

        if self.mac.as_ref().is_none_or(|mac| mac.id != source.id) {
            self.layout = Layout::find(&source.id);
        }

        self.mac = Some(source);

        if secure || confirmed {
            return self.release();
        }

        if (first || was_secure) && self.pending.is_none() {
            self.attempts = 0;
            return self.request(now);
        }

        Vec::new()
    }

    /// 지정을 다시 보낼 시각. 확인을 기다리지 않으면 `None`입니다.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.map(|at| at + RETRY)
    }

    /// 확인이 오지 않았습니다. 횟수가 남았으면 다시 지정하고, 아니면 붙잡아 둔 키를 Mac이 알린 입력 소스대로 보냅니다.
    pub fn expire(&mut self, now: Instant) -> Vec<Vec<u8>> {
        if self.attempts < ATTEMPTS {
            self.pending = None;
            return self.request(now);
        }

        self.release()
    }

    // 고른 언어를 지정합니다. 보안 입력 중이면 Mac이 받지 않으므로 끝날 때 다시 지정합니다.
    // 다른 지정이 확인을 기다리는 중이면 Mac이 곧 그 언어로 바꿀 수 있으므로, 알린 입력 소스와 같아도 지정합니다.
    // Mac이 아직 입력 소스를 알리지 않았으면 키를 붙잡지 않습니다. 접속 직후 잠금 화면의 비밀번호처럼 바로 쳐야 하는 입력이 있고,
    // 알리기까지 2초쯤 걸립니다. 알리면 다시 지정합니다.
    fn request(&mut self, now: Instant) -> Vec<Vec<u8>> {
        let Some(source) = self.selected else {
            return Vec::new();
        };

        match &self.mac {
            None => {}
            Some(mac) if mac.secure => return self.release(),
            Some(mac) if mac.id == source && self.pending.is_none() => return Vec::new(),
            Some(_) => {
                self.attempts += 1;
                self.pending = Some(now);
            }
        }

        vec![InputSource::request(source)]
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

        // 보안 입력 중에는 알린 입력 소스와 관계없이 Mac이 ABC로 받습니다.
        let secure = self.mac.as_ref().is_some_and(|mac| mac.secure);
        let sent = match &self.layout {
            Some(layout) if self.modifiers.is_empty() && !secure => layout.translate(keysym),
            _ => keysym,
        };
        self.pressed.insert(keysym, sent);
        client::key(sent, true).to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::{ATTEMPTS, ENGLISH, Keyboard, RETRY};
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

    fn request(id: &str) -> Vec<Vec<u8>> {
        vec![InputSource::request(id)]
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
    fn keys_typed_before_the_confirmation_wait_and_follow_the_new_source() {
        let mut keyboard = english_mac();
        let now = Instant::now();

        assert_eq!(keyboard.select("ko-KR", now), request(KOREAN));
        assert!(keyboard.key(R, true).is_empty());
        assert!(keyboard.key(R, false).is_empty());
        assert_eq!(
            keyboard.observe(source(KOREAN, false), now),
            vec![key(GIYEOK, true), key(GIYEOK, false)]
        );
    }

    #[test]
    fn unconfirmed_request_retries_then_releases_keys_by_the_mac_source() {
        let mut keyboard = english_mac();
        let now = Instant::now();
        keyboard.select("ko-KR", now);
        keyboard.key(R, true);

        assert_eq!(keyboard.deadline(), Some(now + RETRY));

        for _ in 1..ATTEMPTS {
            assert_eq!(keyboard.expire(now), request(KOREAN));
        }

        assert_eq!(keyboard.expire(now), vec![key(R, true)]);
        assert_eq!(keyboard.deadline(), None);
    }

    // 에이전트는 예전 상태를 늦게 다시 알리기도 합니다. 확인 전에 온 다른 입력 소스로는 키를 내보내지 않습니다.
    #[test]
    fn notice_of_another_source_keeps_the_keys_held() {
        let mut keyboard = english_mac();
        let now = Instant::now();
        keyboard.select("ko-KR", now);
        keyboard.key(R, true);

        assert!(keyboard.observe(source(ENGLISH, false), now).is_empty());
        assert_eq!(keyboard.observe(source(KOREAN, false), now), vec![key(GIYEOK, true)]);
    }

    #[test]
    fn toggling_back_before_the_confirmation_still_requests() {
        let mut keyboard = english_mac();
        let now = Instant::now();
        keyboard.select("ko-KR", now);

        assert_eq!(keyboard.select("en-US", now), request(ENGLISH));
        assert!(keyboard.observe(source(KOREAN, false), now).is_empty());
        assert!(keyboard.deadline().is_some());
        keyboard.observe(source(ENGLISH, false), now);
        assert_eq!(keyboard.deadline(), None);
    }

    // Mac에서 바꾼 입력 소스는 다시 지정하지 않고 따릅니다. 알림마다 지정하면 늦게 온 알림과 서로 다툽니다.
    #[test]
    fn mac_side_change_is_followed() {
        let mut keyboard = korean_mac();

        assert!(keyboard.observe(source(ENGLISH, false), Instant::now()).is_empty());
        assert_eq!(keyboard.key(R, true), vec![key(R, true)]);
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
        let mut keyboard = english_mac();
        let now = Instant::now();
        keyboard.key(R, true);
        keyboard.select("ko-KR", now);

        assert_eq!(keyboard.key(R, false), vec![key(R, false)]);
    }

    #[test]
    fn secure_input_defers_the_request_until_it_ends() {
        let mut keyboard = Keyboard::new();
        let now = Instant::now();
        keyboard.observe(source(ENGLISH, true), now);

        assert!(keyboard.select("ko-KR", now).is_empty());
        assert_eq!(keyboard.key(R, true), vec![key(R, true)]);
        assert_eq!(keyboard.observe(source(ENGLISH, false), now), request(KOREAN));
    }

    // 비밀번호 칸으로 들어갈 때 Mac은 보안 입력이 아닌 ABC를 먼저 알리고, 곧 보안 입력을 켰다 끕니다.
    #[test]
    fn secure_input_flicker_restores_the_selected_language() {
        let mut keyboard = korean_mac();
        let now = Instant::now();

        assert!(keyboard.observe(source(ENGLISH, false), now).is_empty());
        assert!(keyboard.observe(source(ENGLISH, true), now).is_empty());
        assert_eq!(keyboard.observe(source(ENGLISH, false), now), request(KOREAN));
    }

    // 접속 직후 잠금 화면의 비밀번호를 곧바로 칠 수 있어야 합니다.
    #[test]
    fn keys_go_out_as_latin_until_the_mac_reports_its_source() {
        let mut keyboard = Keyboard::new();
        let now = Instant::now();

        assert_eq!(keyboard.select("ko-KR", now), request(KOREAN));
        assert_eq!(keyboard.key(R, true), vec![key(R, true)]);
        assert_eq!(keyboard.deadline(), None);
    }

    #[test]
    fn first_report_requests_the_selected_language_again() {
        let mut keyboard = Keyboard::new();
        let now = Instant::now();
        keyboard.select("ko-KR", now);

        assert_eq!(keyboard.observe(source(ENGLISH, false), now), request(KOREAN));
        assert!(keyboard.key(R, true).is_empty());
        assert_eq!(keyboard.observe(source(KOREAN, false), now), vec![key(GIYEOK, true)]);
    }

    #[test]
    fn selecting_the_current_source_sends_nothing() {
        let mut keyboard = korean_mac();
        assert!(keyboard.select("ko-KR", Instant::now()).is_empty());
        assert_eq!(keyboard.deadline(), None);
    }

    #[test]
    fn shifted_double_consonant_maps_separately() {
        let mut keyboard = korean_mac();
        assert_eq!(keyboard.key(b'R' as u32, true), vec![key(0x0100_3132, true)]);
        assert_eq!(keyboard.key(b'A' as u32, true), vec![key(0x0100_3141, true)]);
    }

    // Mac은 비밀번호 칸에 들어가며 한국어 입력 소스를 보안 입력과 함께 알리기도 합니다. 이때 자모를 보내면 낱자로 들어갑니다.
    #[test]
    fn secure_input_sends_latin_even_while_the_source_reads_korean() {
        let mut keyboard = korean_mac();
        keyboard.observe(source(KOREAN, true), Instant::now());

        assert_eq!(keyboard.key(R, true), vec![key(R, true)]);
    }

    #[test]
    fn languages_without_a_source_type_in_english() {
        let mut keyboard = korean_mac();
        assert_eq!(keyboard.select("de-DE", Instant::now()), request(ENGLISH));
    }

    #[test]
    fn latin_script_tags_type_in_english() {
        let mut keyboard = english_mac();
        assert!(keyboard.select("sr-Latn-RS", Instant::now()).is_empty());
        assert_eq!(
            keyboard.select("sr-RS", Instant::now()),
            request("com.apple.keylayout.Serbian")
        );
    }
}
