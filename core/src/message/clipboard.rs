//! 클립보드 전송 메시지(`0x1f`)입니다.
//!
//! - 내용은 zlib으로 압축한 항목 목록이며, 항목마다 유형 식별자(UTI), 별칭, 데이터가 있습니다.
//! - 텍스트는 `public.utf8-plain-text` 항목으로 주고받습니다. 표준 RFB의 ClientCutText는 요즘 앱에서 붙여넣기가 되지 않습니다.
use super::zlib;
use crate::error::{Error, Result};
use crate::reader::Reader;
use flate2::Compression;
use flate2::Decompress;
use flate2::write::ZlibEncoder;
use std::io::Write;

const TEXT: &str = "public.utf8-plain-text";

const MAX_CONTENT_BYTES: usize = 64 << 20;
const MAX_ITEMS: u32 = 256;
const MAX_ALIASES: u32 = 64;

/// 클립보드 메시지에서 텍스트 항목을 꺼냅니다. 텍스트 항목이 없거나 클립보드가 비어 있으면 `None`입니다.
pub fn text(message: &[u8]) -> Result<Option<String>> {
    let mut reader = Reader::new(message, "클립보드");
    reader.skip(8)?;

    let len = reader.u32()? as usize;
    let compressed_len = reader.u32()? as usize;
    let compressed = reader.bytes(compressed_len)?;

    if len == 0 {
        return Ok(None);
    }

    if len > MAX_CONTENT_BYTES {
        return Err(Error::TooLong {
            what: "클립보드", len
        });
    }

    let content = zlib::inflate(&mut Decompress::new(true), compressed, len, "클립보드")?;
    let mut reader = Reader::new(&content, "클립보드");
    let count = reader.u32()?;

    if count > MAX_ITEMS {
        return Err(Error::Malformed("클립보드"));
    }

    for _ in 0..count {
        let kind = string(&mut reader)?;
        reader.skip(4)?;

        let aliases = reader.u32()?;

        if aliases > MAX_ALIASES {
            return Err(Error::Malformed("클립보드"));
        }

        for _ in 0..aliases * 2 {
            string(&mut reader)?;
        }

        let data = string(&mut reader)?;

        if kind == TEXT.as_bytes() {
            return Ok(Some(String::from_utf8_lossy(data).into_owned()));
        }
    }

    Ok(None)
}

/// 텍스트 항목 하나로 된 클립보드 메시지를 만듭니다.
pub fn message(text: &str) -> Vec<u8> {
    let mut content = Vec::with_capacity(text.len() + TEXT.len() + 24);
    content.extend_from_slice(&1u32.to_be_bytes());
    content.extend_from_slice(&(TEXT.len() as u32).to_be_bytes());
    content.extend_from_slice(TEXT.as_bytes());
    content.extend_from_slice(&[0; 8]);
    content.extend_from_slice(&(text.len() as u32).to_be_bytes());
    content.extend_from_slice(text.as_bytes());

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&content).expect("Vec에 쓰는 압축은 실패하지 않는다");
    let compressed = encoder.finish().expect("Vec에 쓰는 압축은 실패하지 않는다");

    let mut message = vec![0x1f, 0, 0, 0, 0, 0, 0, 0];
    message.extend_from_slice(&(content.len() as u32).to_be_bytes());
    message.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
    message.extend_from_slice(&compressed);
    message
}

fn string<'a>(reader: &mut Reader<'a>) -> Result<&'a [u8]> {
    let len = reader.u32()? as usize;
    reader.bytes(len)
}

#[cfg(test)]
mod tests {
    use super::{message, text};

    #[test]
    fn text_round_trips_through_a_message() {
        assert_eq!(text(&message("안녕, Mac")).unwrap().as_deref(), Some("안녕, Mac"));
    }

    #[test]
    fn empty_clipboard_has_no_text() {
        let empty = [0x1f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(text(&empty).unwrap(), None);
    }

    #[test]
    fn declared_length_beyond_the_limit_is_refused() {
        let mut header = vec![0x1f, 0, 0, 0, 0, 0, 0, 0];
        header.extend_from_slice(&u32::MAX.to_be_bytes());
        header.extend_from_slice(&0u32.to_be_bytes());
        assert!(text(&header).is_err());
    }
}
