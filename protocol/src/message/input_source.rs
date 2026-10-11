use crate::reader::Reader;

// 알림의 플래그 비트입니다. 비밀번호 칸처럼 보안 입력 중이면 Mac이 ABC를 강제합니다.
const SECURE: u32 = 1;

/// Mac의 키보드 입력 소스입니다.
///
/// - 클라이언트는 `0x1a`로 지정하고, 서버는 바뀔 때마다 `0x455` 사각형으로 알립니다. 서버가 스스로 바꿀 때도 알립니다.
/// - 한국어 입력 소스에서는 글자 키를 호환 자모 keysym으로 보내야 Mac이 조합합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
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

    /// 입력 소스를 지정하는 메시지(`0x1a`)를 만듭니다.
    ///
    /// - `1a 00, u16 길이, u16 버전(1), u16 ID 길이, ID`. 길이는 길이 필드 뒤의 바이트 수입니다.
    /// - 유형 뒤의 패딩이 없으면 서버가 이후 메시지의 경계를 잃습니다.
    pub fn request(id: &str) -> Vec<u8> {
        let id = &id.as_bytes()[..id.len().min(u16::MAX as usize - 16)];
        let mut message = vec![0x1a, 0];
        message.extend_from_slice(&((4 + id.len()) as u16).to_be_bytes());
        message.extend_from_slice(&1u16.to_be_bytes());
        message.extend_from_slice(&(id.len() as u16).to_be_bytes());
        message.extend_from_slice(id);
        message
    }
}

#[cfg(test)]
mod tests {
    use super::InputSource;

    const ABC: &str = "com.apple.keylayout.ABC";

    #[test]
    fn request_has_padding_and_lengths() {
        let mut expected = vec![0x1a, 0, 0, 27, 0, 1, 0, 23];
        expected.extend_from_slice(ABC.as_bytes());
        assert_eq!(InputSource::request(ABC), expected);
    }

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
