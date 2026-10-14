use crate::reader::Reader;

// Notice flag bit. During secure input, such as a password field, the Mac forces ABC.
const SECURE: u32 = 1;

/// The Mac's keyboard input source.
///
/// - The server announces every change with a `0x455` rectangle.
/// - The client sets it with `0x1a`, and keys wait for the announcement while the Mac switches.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct InputSource {
    /// Input source ID, e.g. `com.apple.inputmethod.Korean.2SetKorean`
    pub id: String,
    /// Whether secure input (a password field) is active
    pub secure: bool,
}

impl InputSource {
    /// Parses the notice body.
    pub fn parse(body: &[u8]) -> Option<Self> {
        let mut reader = Reader::new(body, "input source");
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
