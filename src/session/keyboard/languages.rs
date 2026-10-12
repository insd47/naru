use super::tables::TABLES;

/// 태블릿 언어와 그 언어로 칠 때 쓰는 Mac 배열
struct Language {
    /// BCP 47 주 언어. 지역이나 문자 체계에 따라 배열이 갈리는 언어는 그것까지(`zh-HK`, `zh-Hant`)
    tag: &'static str,
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

/// 태블릿 언어(BCP 47 태그)로 칠 때 쓰는 Mac 배열의 ID. 라틴 문자를 그대로 보내는 언어면 `None`입니다.
///
/// - 따로 정한 배열이 있으면 그것을, 없으면 Apple이 그 언어로 표시한 배열 가운데 첫 번째를 고릅니다.
/// - 라틴 문자로 쓰는 언어(`sr-Latn`)와, 라틴 키로 조합하는 언어(일본어 로마자, 중국어 병음)는 배열이 없습니다.
pub fn layout(language: &str) -> Option<&'static str> {
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
    let chosen = |key: &str| {
        LANGUAGES
            .iter()
            .find_map(|language| (language.tag == key).then_some(language.layout))
    };

    regional
        .and_then(|regional| chosen(&regional))
        .or_else(|| chosen(&key))
        .or_else(|| {
            TABLES
                .iter()
                .find_map(|table| (table.language == Some(key.as_str())).then_some(table.id))
        })
}

#[cfg(test)]
mod tests {
    use super::{LANGUAGES, layout};
    use crate::session::keyboard::tables::TABLES;

    #[test]
    fn languages_resolve_by_their_own_layouts() {
        assert_eq!(layout("ru-RU"), Some("com.apple.keylayout.Russian"));
        assert_eq!(layout("el"), Some("com.apple.keylayout.Greek"));
        assert_eq!(layout("uz-Cyrl-UZ"), Some("com.apple.keylayout.Uzbek-Cyrillic"));
    }

    #[test]
    fn chosen_layouts_win_over_the_first_layout() {
        assert_eq!(layout("ko-KR"), Some("com.apple.keylayout.2SetHangul"));
        assert_eq!(layout("fa-IR"), Some("com.apple.keylayout.Persian"));
    }

    #[test]
    fn chinese_script_follows_the_region() {
        assert_eq!(layout("zh-CN"), None);
        assert_eq!(layout("zh_Hans_CN"), None);
        assert_eq!(layout("zh-TW"), Some("com.apple.keylayout.ZhuyinBopomofo"));
        assert_eq!(layout("zh-Hant"), Some("com.apple.keylayout.ZhuyinBopomofo"));
        assert_eq!(layout("zh-Hant-HK"), Some("com.apple.keylayout.CangjieKeyboard"));
    }

    #[test]
    fn latin_languages_have_no_layout() {
        assert_eq!(layout("en-US"), None);
        assert_eq!(layout("de-DE"), None);
        assert_eq!(layout("ja-JP"), None);
        assert_eq!(layout("uz"), None);
        assert_eq!(layout("sr-Latn-RS"), None);
        assert_eq!(layout(""), None);
    }

    #[test]
    fn chosen_layouts_exist_in_the_tables() {
        for language in LANGUAGES {
            assert!(
                TABLES.iter().any(|table| table.id == language.layout),
                "{}",
                language.layout
            );
        }
    }
}
