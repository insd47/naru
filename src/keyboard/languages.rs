use super::ABC;
use super::tables::TABLES;

/// A client language and the Mac layout used to type it
struct Language {
    /// BCP 47 primary language, plus the region or script for languages whose layout depends on it (`zh-HK`, `zh-Hant`)
    tag: &'static str,
    layout: &'static str,
}

/// A Mac input method mode and the layout it overrides
struct Mode {
    id: &'static str,
    /// Language compared against the client language, shaped like [`Language::tag`]
    language: &'static str,
    layout: &'static str,
}

// Only layouts used by input methods (Dubeolsik, Zhuyin, Cangjie, Tamil 99) and languages whose first Apple-tagged
// layout is not the default are listed. Other languages are found by the tables' language tags. Dubeolsik, Zhuyin, and
// Cangjie were checked against the layout the input method uses; Tamil 99 is paired by name only. Traditional Chinese
// defaults to Taiwan's Zhuyin, with Cangjie for Hong Kong and Macau.
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

// When the Mac is in one of these modes, client input of the same language is converted with its layout. This serves
// users of non-default layouts such as Sebeolsik or Kana typing. Modes pick their layout at runtime, so the tables
// cannot hold them. Modes that take Latin keys, like Romaji, are left out. The Korean, Traditional Chinese, and
// Japanese Kana modes were checked by reading the current layout after selecting each mode; Tamil 99 is paired by name
// only.
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

// The input method mode each language starts in, for languages typed through one. Korean was checked on a Mac; the
// others are Apple's default modes for their language.
#[rustfmt::skip]
const METHODS: [(&str, &str); 8] = [
    ("ko",      "com.apple.inputmethod.Korean.2SetKorean"),
    ("ja",      "com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese"),
    ("zh-Hans", "com.apple.inputmethod.SCIM.ITABC"),
    ("zh-Hant", "com.apple.inputmethod.TCIM.Zhuyin"),
    ("zh-HK",   "com.apple.inputmethod.TCIM.Cangjie"),
    ("zh-MO",   "com.apple.inputmethod.TCIM.Cangjie"),
    ("vi",      "com.apple.inputmethod.VietnameseIM.VietnameseTelex"),
    ("ta",      "com.apple.inputmethod.Tamil.Tamil99"),
];

/// ID of the Mac layout used to type the client language (a BCP 47 tag), or `None` for languages that send Latin
/// letters as is.
///
/// - When the Mac's reported input source (`mac`) is a mode or layout of the same language, that layout is used, to
///   follow the Mac's layout choice (Sebeolsik, Kana typing).
/// - Otherwise it picks the chosen layout, falling back to the first layout Apple tags with that language.
/// - Languages written in Latin script (`sr-Latn`) and languages composed from Latin keys (Japanese Romaji, Chinese
///   Pinyin) have no layout.
pub fn layout(language: &str, mac: Option<&str>) -> Option<&'static str> {
    let (key, regional) = keys(language)?;
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

// The tag languages are looked up by (`ko`, `zh-Hant`) and its regional form (`zh-HK`), or `None` for languages written
// in Latin script.
fn keys(language: &str) -> Option<(String, Option<String>)> {
    let mut subtags = language.split(['-', '_']);
    let primary = subtags.next()?.to_ascii_lowercase();
    let rest: Vec<&str> = subtags.collect();

    // Chinese without a script subtag takes its script from the region.
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

    // A region-specific mapping (`zh-HK`) wins when present.
    let regional = rest
        .iter()
        .find(|subtag| subtag.len() == 2 && subtag.chars().all(|char| char.is_ascii_alphabetic()))
        .map(|region| format!("{primary}-{}", region.to_ascii_uppercase()));
    let key = match script {
        Some(script) => format!("{primary}-{script}"),
        None => primary,
    };

    Some((key, regional))
}

/// The Mac input source that types the client language, for clients that set the Mac's input source to follow their
/// own.
///
/// - A Mac already on a mode or layout of the same language (Sebeolsik, Kana typing) keeps it.
/// - Languages typed through a Mac input method get that method's default mode; other non-Latin languages get their
///   layout, and Latin languages get ABC.
pub fn source(language: &str, mac: Option<&str>) -> String {
    let Some((key, regional)) = keys(language) else {
        return ABC.to_owned();
    };
    let same_language = |mac: &&str| {
        MODES.iter().any(|mode| mode.id == *mac && mode.language == key)
            || METHODS.iter().any(|(tag, id)| id == mac && *tag == key)
            || TABLES
                .iter()
                .any(|table| table.id == *mac && table.language == Some(key.as_str()))
    };
    let method = |key: &str| METHODS.iter().find_map(|(tag, id)| (*tag == key).then_some(*id));

    mac.filter(same_language)
        .or_else(|| regional.as_deref().and_then(method))
        .or_else(|| method(&key))
        .or_else(|| layout(language, None))
        .unwrap_or(ABC)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{LANGUAGES, MODES, layout, source};
    use crate::keyboard::tables::TABLES;

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

    // A Mac mode of the same language picks its layout; one of another language leaves the client language's default
    // layout.
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

    #[test]
    fn sources_follow_the_language_and_keep_the_macs_own_mode() {
        assert_eq!(source("ko-KR", None), "com.apple.inputmethod.Korean.2SetKorean");
        assert_eq!(
            source("en-US", Some("com.apple.inputmethod.Korean.2SetKorean")),
            "com.apple.keylayout.ABC"
        );
        assert_eq!(source("ru", None), "com.apple.keylayout.Russian");
        assert_eq!(source("zh-HK", None), "com.apple.inputmethod.TCIM.Cangjie");

        let three_set = "com.apple.inputmethod.Korean.3SetKorean";
        assert_eq!(source("ko", Some(three_set)), three_set);
        assert_eq!(
            source("ja", Some(three_set)),
            "com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese"
        );
    }
}
