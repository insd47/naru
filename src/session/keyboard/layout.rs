use super::tables::{TABLES, Table};
use std::collections::HashMap;

const ABC: &str = "com.apple.keylayout.ABC";
const EMPTY: char = '\u{FFFD}';

/// 라틴 문자를 칠 수 없는 Mac 배열에서, ABC 자리의 글자를 그 배열이 같은 자리에서 내는 글자로 바꿉니다.
///
/// - 에이전트는 받은 글자를 Mac의 지금 배열에서 키로 되돌려 입력기에 넣습니다. 2벌식이면 `r` 대신 `ㄱ`을 보내야 조합됩니다.
/// - 라틴 배열(ABC, 독일어 등)은 표가 없습니다. 글자를 그대로 보내면 Mac이 그 글자를 내는 키를 찾습니다.
pub struct Layout(HashMap<u32, u32>);

impl Layout {
    /// Mac 배열의 변환 표. 바꿀 글자가 없는 라틴 배열이거나 모르는 배열이면 `None`입니다.
    pub fn find(id: &str) -> Option<Self> {
        let abc = table(ABC)?;
        let target = table(id)?;

        let pairs = abc
            .base
            .chars()
            .zip(target.base.chars())
            .chain(abc.shift.chars().zip(target.shift.chars()));
        let map = pairs
            .filter(|&(latin, char)| latin != char && char != EMPTY)
            .map(|(latin, char)| (keysym(latin), keysym(char)))
            .collect::<HashMap<_, _>>();

        (!map.is_empty()).then_some(Self(map))
    }

    /// 이 배열에서 보낼 keysym. 바꿀 것이 없으면 받은 그대로입니다.
    pub fn translate(&self, keysym: u32) -> u32 {
        self.0.get(&keysym).copied().unwrap_or(keysym)
    }
}

fn table(id: &str) -> Option<&'static Table> {
    TABLES.iter().find(|table| table.id == id)
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
    use super::{ABC, Layout, keysym, table};
    use crate::session::keyboard::tables::TABLES;

    // 생성한 표가 예전에 손으로 쓴 2벌식 표와 같은 결과를 내는지 확인합니다.
    #[test]
    fn two_set_matches_the_dubeolsik_chart() {
        let layout = Layout::find("com.apple.keylayout.2SetHangul").unwrap();
        let chart = "qㅂwㅈeㄷrㄱtㅅyㅛuㅕiㅑoㅐpㅔaㅁsㄴdㅇfㄹgㅎhㅗjㅓkㅏlㅣzㅋxㅌcㅊvㅍbㅠnㅜmㅡ\
                     QㅃWㅉEㄸRㄲTㅆOㅒPㅖAㅁHㅗMㅡ";
        let chars: Vec<char> = chart.chars().collect();

        for pair in chars.chunks(2) {
            assert_eq!(layout.translate(keysym(pair[0])), keysym(pair[1]), "{}", pair[0]);
        }
    }

    #[test]
    fn digits_and_punctuation_stay() {
        let layout = Layout::find("com.apple.keylayout.2SetHangul").unwrap();
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
    fn latin_and_unknown_layouts_have_no_table() {
        assert!(Layout::find(ABC).is_none());
        assert!(Layout::find("com.apple.keylayout.German").is_none());
        assert!(Layout::find("com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese").is_none());
    }

    #[test]
    fn rows_line_up_with_abc() {
        let width = table(ABC).unwrap().base.chars().count();

        for table in TABLES {
            assert_eq!(table.base.chars().count(), width, "{}", table.id);
            assert_eq!(table.shift.chars().count(), width, "{}", table.id);
        }
    }

    #[test]
    fn traditional_chinese_layouts_translate() {
        let zhuyin = Layout::find("com.apple.keylayout.ZhuyinBopomofo").unwrap();
        assert_eq!(zhuyin.translate(u32::from(b'q')), keysym('ㄆ'));
    }
}
