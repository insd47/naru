use std::collections::HashMap;

// macOS 배열에서 뽑은 표. `tools/layouts.swift`가 만듭니다.
const LAYOUTS: &str = include_str!("layouts.txt");

const ABC: &str = "com.apple.keylayout.ABC";
const EMPTY: char = '\u{FFFD}';

// 입력 모드가 덮어쓰는 배열. 모드는 배열을 실행 중에 고르므로 표에 담기지 않습니다.
// 한국어 입력기 번들이 이 다섯 배열을 자기 것으로 싣고 있고, 2벌식은 Mac에서 직접 확인했습니다.
const MODES: [(&str, &str); 5] = [
    (
        "com.apple.inputmethod.Korean.2SetKorean",
        "com.apple.keylayout.2SetHangul",
    ),
    (
        "com.apple.inputmethod.Korean.3SetKorean",
        "com.apple.keylayout.3SetHangul",
    ),
    (
        "com.apple.inputmethod.Korean.390Sebulshik",
        "com.apple.keylayout.390Hangul",
    ),
    (
        "com.apple.inputmethod.Korean.GongjinCheongRomaja",
        "com.apple.keylayout.GJCRomaja",
    ),
    (
        "com.apple.inputmethod.Korean.HNCRomaja",
        "com.apple.keylayout.HNCRomaja",
    ),
];

/// 라틴 문자를 칠 수 없는 Mac 배열에서, ABC 자리의 글자를 그 배열이 같은 자리에서 내는 글자로 바꿉니다.
///
/// - 에이전트는 받은 글자를 Mac의 지금 배열에서 키로 되돌려 입력기에 넣습니다. 2벌식이면 `r` 대신 `ㄱ`을 보내야 조합됩니다.
/// - 라틴 배열(ABC, 독일어 등)은 표가 없습니다. 글자를 그대로 보내면 Mac이 그 글자를 내는 키를 찾습니다.
pub struct Layout(HashMap<u32, u32>);

impl Layout {
    /// Mac 입력 소스의 변환 표. 바꿀 글자가 없는 라틴 배열이거나 모르는 입력 소스면 `None`입니다.
    pub fn find(source: &str) -> Option<Self> {
        let id = MODES
            .iter()
            .find_map(|&(mode, layout)| (mode == source).then_some(layout))
            .unwrap_or(source);
        let (abc_base, abc_shift) = rows(ABC)?;
        let (base, shift) = rows(id)?;

        let pairs = abc_base
            .chars()
            .zip(base.chars())
            .chain(abc_shift.chars().zip(shift.chars()));
        let table = pairs
            .filter(|&(latin, target)| latin != target && target != EMPTY)
            .map(|(latin, target)| (keysym(latin), keysym(target)))
            .collect::<HashMap<_, _>>();

        (!table.is_empty()).then_some(Self(table))
    }

    /// 이 배열에서 보낼 keysym. 바꿀 것이 없으면 받은 그대로입니다.
    pub fn translate(&self, keysym: u32) -> u32 {
        self.0.get(&keysym).copied().unwrap_or(keysym)
    }
}

// 표에서 배열 한 줄의 기본 열과 Shift 열
fn rows(id: &str) -> Option<(&'static str, &'static str)> {
    LAYOUTS.lines().filter(|line| !line.starts_with('#')).find_map(|line| {
        let mut fields = line.split('\t');
        (fields.next()? == id).then_some(())?;
        Some((fields.next()?, fields.next()?))
    })
}

// 라틴-1은 문자 코드 그대로, 그 밖은 유니코드 keysym입니다.
fn keysym(char: char) -> u32 {
    match u32::from(char) {
        code @ (0x20..=0x7e | 0xa0..=0xff) => code,
        code => 0x0100_0000 + code,
    }
}

#[cfg(test)]
mod tests {
    use super::{ABC, LAYOUTS, Layout, MODES, keysym, rows};

    // 생성한 표가 예전에 손으로 쓴 2벌식 표와 같은 결과를 내는지 확인합니다.
    #[test]
    fn two_set_matches_the_dubeolsik_chart() {
        let layout = Layout::find("com.apple.inputmethod.Korean.2SetKorean").unwrap();
        let chart = "qㅂwㅈeㄷrㄱtㅅyㅛuㅕiㅑoㅐpㅔaㅁsㄴdㅇfㄹgㅎhㅗjㅓkㅏlㅣzㅋxㅌcㅊvㅍbㅠnㅜmㅡ\
                     QㅃWㅉEㄸRㄲTㅆOㅒPㅖAㅁHㅗMㅡ";
        let chars: Vec<char> = chart.chars().collect();

        for pair in chars.chunks(2) {
            assert_eq!(layout.translate(keysym(pair[0])), keysym(pair[1]), "{}", pair[0]);
        }
    }

    #[test]
    fn digits_and_punctuation_stay() {
        let layout = Layout::find("com.apple.inputmethod.Korean.2SetKorean").unwrap();
        assert_eq!(layout.translate(u32::from(b'1')), u32::from(b'1'));
        assert_eq!(layout.translate(u32::from(b',')), u32::from(b','));
    }

    #[test]
    fn plain_layouts_translate_by_position() {
        let layout = Layout::find("com.apple.keylayout.Russian").unwrap();
        assert_eq!(layout.translate(u32::from(b'q')), keysym('й'));
        assert_eq!(layout.translate(u32::from(b'Q')), keysym('Й'));
    }

    // 키릴 문자처럼 ABC에 없는 글자는 태블릿 배열이 이미 바꾼 것이므로 그대로 보냅니다.
    #[test]
    fn characters_outside_abc_pass_through() {
        let layout = Layout::find("com.apple.keylayout.Russian").unwrap();
        assert_eq!(layout.translate(keysym('ж')), keysym('ж'));
    }

    #[test]
    fn latin_and_unknown_sources_have_no_table() {
        assert!(Layout::find(ABC).is_none());
        assert!(Layout::find("com.apple.keylayout.German").is_none());
        assert!(Layout::find("com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese").is_none());
    }

    #[test]
    fn every_mode_has_its_layout_and_rows_line_up() {
        for (_, layout) in MODES {
            assert!(rows(layout).is_some(), "{layout}");
        }

        let width = rows(ABC).unwrap().0.chars().count();
        for line in LAYOUTS.lines().filter(|line| !line.starts_with('#')) {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.len(), 3, "{}", fields[0]);
            assert!(
                fields[1..].iter().all(|row| row.chars().count() == width),
                "{}",
                fields[0]
            );
        }
    }
}
