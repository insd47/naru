use crate::reader::Reader;

// 알림의 플래그 비트입니다. 비밀번호 칸처럼 보안 입력 중이면 Mac이 ABC를 강제합니다.
const SECURE: u32 = 1;

/// Mac의 키보드 입력 소스입니다.
///
/// - 서버는 바뀔 때마다 `0x455` 사각형으로 알립니다.
/// - 클라이언트가 `0x1a`로 지정할 수도 있지만 입력기가 쓰는 배열은 이전 것에 남으므로 쓰지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct InputSource {
    /// 입력 소스 ID(예: `com.apple.inputmethod.Korean.2SetKorean`)
    pub id: String,
    /// 보안 입력(비밀번호 칸) 중인지 여부
    pub secure: bool,
}

impl InputSource {
    /// 알림 본문을 해석합니다. `u16` 버전, `u32` 플래그, `u16` 길이, ID 순입니다.
    pub fn parse(body: &[u8]) -> Option<Self> {
        let mut reader = Reader::new(body, "입력 소스");
        reader.skip(2).ok()?;
        let flags = reader.u32().ok()?;
        let len = reader.u16().ok()? as usize;
        let id = reader.bytes(len).ok()?;

        Some(Self {
            id: String::from_utf8_lossy(id).into_owned(),
            secure: flags & SECURE != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::InputSource;

    const ABC: &str = "com.apple.keylayout.ABC";

    #[test]
    fn notice_carries_the_secure_input_flag() {
        let notice = |flags: u32| [&[0, 1][..], &flags.to_be_bytes(), &23u16.to_be_bytes(), ABC.as_bytes()].concat();

        let secure = InputSource::parse(&notice(1)).unwrap();
        assert_eq!((secure.id.as_str(), secure.secure), (ABC, true));
        assert!(!InputSource::parse(&notice(0)).unwrap().secure);
    }

    #[test]
    fn truncated_notice_is_ignored() {
        assert_eq!(InputSource::parse(&[0, 1, 0, 0, 0, 0, 0, 23, b'c']), None);
    }
}
