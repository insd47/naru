use super::tables::TABLES;

/// 태블릿 언어에 대응하는 Mac 입력 소스
struct Language {
    /// BCP 47 주 언어, 문자 체계가 갈리는 언어는 문자 체계까지(`zh-Hans`)
    tag: &'static str,
    source: &'static str,
}

/// 입력 모드와 그 모드가 덮어쓰는 배열
pub struct Mode {
    pub id: &'static str,
    pub layout: &'static str,
}

// Mac 입력기를 거치는 언어와, Apple이 그 언어로 표시한 배열 가운데 첫 번째가 기본 배열이 아닌 언어입니다.
// 나머지 언어는 표의 언어 태그로 찾습니다. 기기에서 확인한 것은 한국어뿐입니다.
// 일본어 가나 입력과 중국어 번체(주음, 창힐)는 모드가 덮어쓰는 배열을 확인하지 못해 두지 않습니다.
const LANGUAGES: [Language; 7] = [
    Language {
        tag: "ko",
        source: "com.apple.inputmethod.Korean.2SetKorean",
    },
    Language {
        tag: "ja",
        source: "com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese",
    },
    Language {
        tag: "zh-Hans",
        source: "com.apple.inputmethod.SCIM.ITABC",
    },
    Language {
        tag: "vi",
        source: "com.apple.inputmethod.VietnameseIM.VietnameseTelex",
    },
    Language {
        tag: "ta",
        source: "com.apple.inputmethod.Tamil.Tamil99",
    },
    Language {
        tag: "fa",
        source: "com.apple.keylayout.Persian",
    },
    Language {
        tag: "iu",
        source: "com.apple.keylayout.Inuktitut-Nunavut",
    },
];

// 모드는 배열을 실행 중에 고르므로 표에 담기지 않습니다. 로마자, 병음, Telex처럼 라틴 키를 그대로 받는 모드는 두지 않습니다.
// 한국어 입력기 번들이 다섯 배열을 자기 것으로 싣고 있고, 2벌식은 Mac에서 직접 확인했습니다. 타밀 99는 이름으로 짝지은 것입니다.
pub const MODES: [Mode; 6] = [
    Mode {
        id: "com.apple.inputmethod.Korean.2SetKorean",
        layout: "com.apple.keylayout.2SetHangul",
    },
    Mode {
        id: "com.apple.inputmethod.Korean.3SetKorean",
        layout: "com.apple.keylayout.3SetHangul",
    },
    Mode {
        id: "com.apple.inputmethod.Korean.390Sebulshik",
        layout: "com.apple.keylayout.390Hangul",
    },
    Mode {
        id: "com.apple.inputmethod.Korean.GongjinCheongRomaja",
        layout: "com.apple.keylayout.GJCRomaja",
    },
    Mode {
        id: "com.apple.inputmethod.Korean.HNCRomaja",
        layout: "com.apple.keylayout.HNCRomaja",
    },
    Mode {
        id: "com.apple.inputmethod.Tamil.Tamil99",
        layout: "com.apple.keylayout.Tamil99",
    },
];

/// 태블릿 언어(BCP 47 태그)에 대응하는 Mac 입력 소스. 없으면 `None`이고, 세션은 ABC로 칩니다.
///
/// - 따로 정한 대응이 있으면 그것을, 없으면 Apple이 그 언어로 표시한 배열 가운데 첫 번째를 고릅니다.
/// - 라틴 문자로 쓰는 언어(`sr-Latn`)는 대응하지 않습니다. 태블릿 배열이 낸 글자를 그대로 보내면 됩니다.
pub fn source(language: &str) -> Option<&'static str> {
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

    let key = match script {
        Some(script) => format!("{primary}-{script}"),
        None => primary,
    };

    LANGUAGES
        .iter()
        .find_map(|language| (language.tag == key).then_some(language.source))
        .or_else(|| {
            TABLES
                .iter()
                .find_map(|table| (table.language == Some(key.as_str())).then_some(table.id))
        })
}

#[cfg(test)]
mod tests {
    use super::{LANGUAGES, MODES, source};
    use crate::session::keyboard::tables::TABLES;

    #[test]
    fn languages_resolve_by_their_own_layouts() {
        assert_eq!(source("ru-RU"), Some("com.apple.keylayout.Russian"));
        assert_eq!(source("el"), Some("com.apple.keylayout.Greek"));
        assert_eq!(source("uz-Cyrl-UZ"), Some("com.apple.keylayout.Uzbek-Cyrillic"));
    }

    #[test]
    fn chosen_sources_win_over_the_first_layout() {
        assert_eq!(source("ko-KR"), Some("com.apple.inputmethod.Korean.2SetKorean"));
        assert_eq!(source("fa-IR"), Some("com.apple.keylayout.Persian"));
    }

    #[test]
    fn chinese_script_follows_the_region() {
        assert_eq!(source("zh-CN"), Some("com.apple.inputmethod.SCIM.ITABC"));
        assert_eq!(source("zh_Hans_CN"), Some("com.apple.inputmethod.SCIM.ITABC"));
        assert_eq!(source("zh-TW"), None);
        assert_eq!(source("zh-Hant"), None);
    }

    #[test]
    fn latin_languages_have_no_source() {
        assert_eq!(source("en-US"), None);
        assert_eq!(source("de-DE"), None);
        assert_eq!(source("uz"), None);
        assert_eq!(source("sr-Latn-RS"), None);
        assert_eq!(source(""), None);
    }

    // 직접 고른 배열은 사용자가 고를 수 있는 배열이어야 하고, 모드가 덮어쓰는 배열은 표에 있어야 합니다.
    #[test]
    fn chosen_layouts_exist_in_the_tables() {
        let selectable = |id: &str| TABLES.iter().any(|table| table.id == id && table.language.is_some());

        for language in LANGUAGES
            .iter()
            .filter(|language| language.source.starts_with("com.apple.keylayout."))
        {
            assert!(selectable(language.source), "{}", language.source);
        }

        for mode in MODES {
            assert!(TABLES.iter().any(|table| table.id == mode.layout), "{}", mode.layout);
        }
    }
}
