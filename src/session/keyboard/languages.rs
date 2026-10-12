use super::tables::TABLES;

/// 태블릿 언어와 그 언어로 칠 때 쓰는 Mac 배열
struct Language {
    /// BCP 47 주 언어. 지역이나 문자 체계에 따라 배열이 갈리는 언어는 그것까지(`zh-HK`, `zh-Hant`)
    tag: &'static str,
    layout: &'static str,
}

/// Mac 입력기의 모드와 그 모드가 덮어쓰는 배열
struct Mode {
    id: &'static str,
    /// 태블릿 언어와 견주는 언어. [`Language::tag`]와 같은 모양입니다.
    language: &'static str,
    layout: &'static str,
}

// 입력기만 쓰는 배열(2벌식, 주음, 창힐, 타밀 99)과, Apple이 그 언어로 표시한 배열 가운데 첫 번째가 기본 배열이 아닌 언어만 둡니다.
// 나머지 언어는 표의 언어 태그로 찾습니다. 2벌식, 주음, 창힐은 macOS 27.2에서 입력기가 쓰는 배열을 읽어 확인했고, 타밀 99는 이름으로 짝지은 것입니다.
// 번체 중국어는 대만의 주음을 기본으로, 홍콩과 마카오는 창힐로 둡니다.
#[rustfmt::skip]
const LANGUAGES: [Language; 7] = [
    Language { tag: "ko",      layout: "com.apple.keylayout.2SetHangul" },
    Language { tag: "zh-Hant", layout: "com.apple.keylayout.ZhuyinBopomofo" },
    Language { tag: "zh-HK",   layout: "com.apple.keylayout.CangjieKeyboard" },
    Language { tag: "zh-MO",   layout: "com.apple.keylayout.CangjieKeyboard" },
    Language { tag: "ta",      layout: "com.apple.keylayout.Tamil99" },
    Language { tag: "fa",      layout: "com.apple.keylayout.Persian" },
    Language { tag: "iu",      layout: "com.apple.keylayout.Inuktitut-Nunavut" },
];

// Mac이 이 모드에 있으면 같은 언어의 태블릿 입력을 그 배열로 바꿉니다. 3벌식이나 가나 입력처럼 기본과 다른 배열을 쓰는 사용자를 위한 것입니다.
// 모드는 배열을 실행 중에 고르므로 표에 담기지 않습니다. 로마자처럼 라틴 키를 받는 모드는 두지 않습니다.
// 한국어, 번체 중국어, 일본어 가나 입력은 macOS 27.2에서 메뉴로 모드를 고른 뒤 지금 배열을 읽어 확인했습니다. 타밀 99는 이름으로 짝지은 것입니다.
#[rustfmt::skip]
const MODES: [Mode; 12] = [
    Mode { id: "com.apple.inputmethod.Korean.2SetKorean",                         language: "ko",      layout: "com.apple.keylayout.2SetHangul" },
    Mode { id: "com.apple.inputmethod.Korean.3SetKorean",                         language: "ko",      layout: "com.apple.keylayout.3SetHangul" },
    Mode { id: "com.apple.inputmethod.Korean.390Sebulshik",                       language: "ko",      layout: "com.apple.keylayout.390Hangul" },
    Mode { id: "com.apple.inputmethod.Korean.GongjinCheongRomaja",                language: "ko",      layout: "com.apple.keylayout.GJCRomaja" },
    Mode { id: "com.apple.inputmethod.Korean.HNCRomaja",                          language: "ko",      layout: "com.apple.keylayout.HNCRomaja" },
    Mode { id: "com.apple.inputmethod.TCIM.Zhuyin",                               language: "zh-Hant", layout: "com.apple.keylayout.ZhuyinBopomofo" },
    Mode { id: "com.apple.inputmethod.TCIM.Cangjie",                              language: "zh-Hant", layout: "com.apple.keylayout.CangjieKeyboard" },
    Mode { id: "com.apple.inputmethod.TCIM.Jianyi",                               language: "zh-Hant", layout: "com.apple.keylayout.CangjieKeyboard" },
    Mode { id: "com.apple.inputmethod.Kotoeri.KanaTyping.Japanese",               language: "ja",      layout: "com.apple.keylayout.KANA" },
    Mode { id: "com.apple.inputmethod.Kotoeri.KanaTyping.Japanese.Katakana",      language: "ja",      layout: "com.apple.keylayout.KANA" },
    Mode { id: "com.apple.inputmethod.Kotoeri.KanaTyping.Japanese.HalfWidthKana", language: "ja",      layout: "com.apple.keylayout.KANA" },
    Mode { id: "com.apple.inputmethod.Tamil.Tamil99",                             language: "ta",      layout: "com.apple.keylayout.Tamil99" },
];

/// 태블릿 언어(BCP 47 태그)로 칠 때 쓰는 Mac 배열의 ID. 라틴 문자를 그대로 보내는 언어면 `None`입니다.
///
/// - Mac이 알린 입력 소스(`mac`)가 같은 언어의 모드나 배열이면 그 배열을 씁니다. Mac의 배열 선택(3벌식, 가나 입력)을 따르기 위해서입니다.
/// - 아니면 따로 정한 배열을, 없으면 Apple이 그 언어로 표시한 배열 가운데 첫 번째를 고릅니다.
/// - 라틴 문자로 쓰는 언어(`sr-Latn`)와, 라틴 키로 조합하는 언어(일본어 로마자, 중국어 병음)는 배열이 없습니다.
pub fn layout(language: &str, mac: Option<&str>) -> Option<&'static str> {
    let mut subtags = language.split(['-', '_']);
    let primary = subtags.next()?.to_ascii_lowercase();
    let rest: Vec<&str> = subtags.collect();

    // 중국어는 태그에 문자 체계가 없으면 지역으로 정합니다.
    let script = rest
        .iter()
        .find(|subtag| subtag.len() == 4 && subtag.chars().all(|char| char.is_ascii_alphabetic()))
        .map(|subtag| subtag[..1].to_ascii_uppercase() + &subtag[1..].to_ascii_lowercase())
        .or_else(|| {
            let traditional = rest.iter().any(|subtag| {
                ["TW", "HK", "MO"]
                    .iter()
                    .any(|region| subtag.eq_ignore_ascii_case(region))
            });
            (primary == "zh").then(|| if traditional { "Hant" } else { "Hans" }.to_owned())
        });

    if script.as_deref() == Some("Latn") {
        return None;
    }

    // 지역으로 정한 대응(`zh-HK`)이 있으면 먼저 씁니다.
    let regional = rest
        .iter()
        .find(|subtag| subtag.len() == 2 && subtag.chars().all(|char| char.is_ascii_alphabetic()))
        .map(|region| format!("{primary}-{}", region.to_ascii_uppercase()));
    let key = match script {
        Some(script) => format!("{primary}-{script}"),
        None => primary,
    };
    let current = mac.and_then(|mac| {
        let mode = MODES
            .iter()
            .find(|mode| mode.id == mac && mode.language == key)
            .map(|mode| mode.layout);
        mode.or_else(|| {
            TABLES
                .iter()
                .find(|table| table.id == mac && table.language == Some(key.as_str()))
                .map(|table| table.id)
        })
    });
    let chosen = |key: &str| {
        LANGUAGES
            .iter()
            .find_map(|language| (language.tag == key).then_some(language.layout))
    };

    current
        .or_else(|| regional.and_then(|regional| chosen(&regional)))
        .or_else(|| chosen(&key))
        .or_else(|| {
            TABLES
                .iter()
                .find_map(|table| (table.language == Some(key.as_str())).then_some(table.id))
        })
}

#[cfg(test)]
mod tests {
    use super::{LANGUAGES, MODES, layout};
    use crate::session::keyboard::tables::TABLES;

    #[test]
    fn languages_resolve_by_their_own_layouts() {
        assert_eq!(layout("ru-RU", None), Some("com.apple.keylayout.Russian"));
        assert_eq!(layout("el", None), Some("com.apple.keylayout.Greek"));
        assert_eq!(layout("uz-Cyrl-UZ", None), Some("com.apple.keylayout.Uzbek-Cyrillic"));
    }

    #[test]
    fn chosen_layouts_win_over_the_first_layout() {
        assert_eq!(layout("ko-KR", None), Some("com.apple.keylayout.2SetHangul"));
        assert_eq!(layout("fa-IR", None), Some("com.apple.keylayout.Persian"));
    }

    #[test]
    fn chinese_script_follows_the_region() {
        assert_eq!(layout("zh-CN", None), None);
        assert_eq!(layout("zh_Hans_CN", None), None);
        assert_eq!(layout("zh-TW", None), Some("com.apple.keylayout.ZhuyinBopomofo"));
        assert_eq!(layout("zh-Hant", None), Some("com.apple.keylayout.ZhuyinBopomofo"));
        assert_eq!(layout("zh-Hant-HK", None), Some("com.apple.keylayout.CangjieKeyboard"));
    }

    #[test]
    fn latin_languages_have_no_layout() {
        assert_eq!(layout("en-US", None), None);
        assert_eq!(layout("de-DE", None), None);
        assert_eq!(layout("ja-JP", None), None);
        assert_eq!(layout("uz", None), None);
        assert_eq!(layout("sr-Latn-RS", None), None);
        assert_eq!(layout("", None), None);
    }

    // Mac의 모드가 같은 언어면 그 배열을 따르고, 다른 언어면 태블릿 언어의 기본 배열을 씁니다.
    #[test]
    fn mac_mode_of_the_same_language_picks_the_layout() {
        let three_set = Some("com.apple.inputmethod.Korean.3SetKorean");
        assert_eq!(layout("ko-KR", three_set), Some("com.apple.keylayout.3SetHangul"));
        assert_eq!(layout("ru-RU", three_set), Some("com.apple.keylayout.Russian"));

        let kana = Some("com.apple.inputmethod.Kotoeri.KanaTyping.Japanese");
        assert_eq!(layout("ja-JP", kana), Some("com.apple.keylayout.KANA"));
        assert_eq!(layout("en-US", kana), None);
        assert_eq!(
            layout("ja-JP", Some("com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese")),
            None
        );

        assert_eq!(
            layout("zh-HK", Some("com.apple.inputmethod.TCIM.Zhuyin")),
            Some("com.apple.keylayout.ZhuyinBopomofo")
        );
        assert_eq!(
            layout("ru-RU", Some("com.apple.keylayout.Russian-Phonetic")),
            Some("com.apple.keylayout.Russian-Phonetic")
        );
        assert_eq!(
            layout("ko-KR", Some("com.apple.keylayout.ABC")),
            Some("com.apple.keylayout.2SetHangul")
        );
    }

    #[test]
    fn chosen_layouts_exist_in_the_tables() {
        for mode in MODES {
            assert!(TABLES.iter().any(|table| table.id == mode.layout), "{}", mode.layout);
        }

        for language in LANGUAGES {
            assert!(
                TABLES.iter().any(|table| table.id == language.layout),
                "{}",
                language.layout
            );
        }
    }
}
