use super::ABC;
use super::tables::{TABLES, Table};

const EMPTY: char = '\u{FFFD}';

/// For a Mac layout that cannot type Latin letters, converts the character at an ABC position to the one that layout
/// produces at the same position.
///
/// - The agent maps each received character back to a key in the Mac's current layout and feeds it to the input method.
///   With Dubeolsik, `ㄱ` must be sent instead of `r` to compose.
/// - Latin layouts (ABC, German, and so on) have no table. Sent as is, the Mac finds the key that produces the
///   character.
pub struct Layout {
    abc: &'static Table,
    target: &'static Table,
}

impl Layout {
    /// Conversion table for a Mac layout, or `None` for a Latin layout with nothing to convert or an unknown layout.
    pub fn find(id: &str) -> Option<Self> {
        (id != ABC).then_some(())?;

        Some(Self {
            abc: table(ABC)?,
            target: table(id)?,
        })
    }

    /// The keysym to send in this layout, unchanged when there is nothing to convert
    pub fn translate(&self, latin: u32) -> u32 {
        let rows = [(self.abc.base, self.target.base), (self.abc.shift, self.target.shift)];

        rows.iter()
            .find_map(|(abc, target)| {
                let column = abc.chars().position(|char| keysym(char) == latin)?;
                target.chars().nth(column)
            })
            .filter(|&char| char != EMPTY)
            .map_or(latin, keysym)
    }
}

fn table(id: &str) -> Option<&'static Table> {
    TABLES.iter().find(|table| table.id == id)
}

// Latin-1 maps to its character code; anything else is a Unicode keysym.
fn keysym(char: char) -> u32 {
    match u32::from(char) {
        code @ (0x20..=0x7e | 0xa0..=0xff) => code,
        code => 0x0100_0000 + code,
    }
}

#[cfg(test)]
mod tests {
    use super::{Layout, keysym, table};
    use crate::keyboard::ABC;
    use crate::keyboard::tables::TABLES;

    // Checks the generated table against the Dubeolsik chart.
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

    // Characters outside ABC, like Cyrillic, were already converted by the client's layout, so they pass through.
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
