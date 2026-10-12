//! The clipboard transfer message (`0x1f`).
//!
//! - The content is a zlib-compressed list of items, each with a type identifier (UTI), aliases, and data.
//! - Text travels as a `public.utf8-plain-text` item, because standard RFB ClientCutText does not paste in current apps.
use super::zlib;
use crate::error::{Error, Result};
use crate::reader::Reader;
use flate2::{Compress, Compression, Decompress, FlushCompress};

const TEXT: &str = "public.utf8-plain-text";

const MAX_CONTENT_BYTES: usize = 64 << 20;
const MAX_ITEMS: u32 = 256;
const MAX_ALIASES: u32 = 64;

/// Extracts the text item from a clipboard message, or `None` when there is no text item or the clipboard is empty.
pub fn text(message: &[u8]) -> Result<Option<String>> {
    let mut reader = Reader::new(message, "clipboard");
    reader.skip(8)?;

    let len = reader.u32()? as usize;
    let compressed_len = reader.u32()? as usize;
    let compressed = reader.bytes(compressed_len)?;

    if len == 0 {
        return Ok(None);
    }

    if len > MAX_CONTENT_BYTES {
        return Err(Error::TooLong { what: "clipboard", len });
    }

    let content = zlib::inflate(&mut Decompress::new(true), compressed, len, "clipboard")?;
    let mut reader = Reader::new(&content, "clipboard");
    let count = reader.u32()?;

    if count > MAX_ITEMS {
        return Err(Error::Malformed("clipboard"));
    }

    for _ in 0..count {
        let kind = string(&mut reader)?;
        reader.skip(4)?;

        let aliases = reader.u32()?;

        if aliases > MAX_ALIASES {
            return Err(Error::Malformed("clipboard"));
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

/// Builds a clipboard message with a single text item, failing with `TooLong` past the same size limit used when receiving.
pub fn message(text: &str) -> Result<Vec<u8>> {
    let len = text.len() + TEXT.len() + 24;

    if len > MAX_CONTENT_BYTES {
        return Err(Error::TooLong { what: "clipboard", len });
    }

    let mut content = Vec::with_capacity(len);
    content.extend_from_slice(&1u32.to_be_bytes());
    content.extend_from_slice(&length(TEXT.len()));
    content.extend_from_slice(TEXT.as_bytes());
    content.extend_from_slice(&[0; 8]);
    content.extend_from_slice(&length(text.len()));
    content.extend_from_slice(text.as_bytes());

    // End with a sync flush and no final block, like Apple's client. With a final block the Mac drops the content and just clears the clipboard.
    let mut compressor = Compress::new(Compression::default(), true);
    let mut compressed = Vec::with_capacity(content.len() + 64);

    while (compressor.total_in() as usize) < content.len() || compressed.len() == compressed.capacity() {
        compressed.reserve(64);
        compressor
            .compress_vec(
                &content[compressor.total_in() as usize..],
                &mut compressed,
                FlushCompress::Sync,
            )
            .expect("in-memory compression does not fail");
    }

    let mut message = vec![0x1f, 0, 0, 0, 0, 0, 0, 0];
    message.extend_from_slice(&length(content.len()));
    message.extend_from_slice(&length(compressed.len()));
    message.extend_from_slice(&compressed);
    Ok(message)
}

// Length fields are `u32`. The content was checked against [`MAX_CONTENT_BYTES`], and compression does not grow it past that.
fn length(len: usize) -> [u8; 4] {
    u32::try_from(len)
        .expect("within the clipboard size limit")
        .to_be_bytes()
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
        assert_eq!(
            text(&message("안녕, Mac").unwrap()).unwrap().as_deref(),
            Some("안녕, Mac")
        );
    }

    // Apple's client also ends the compressed body with a sync flush (`00 00 ff ff`).
    #[test]
    fn content_ends_with_a_sync_flush() {
        assert!(message("안녕, Mac").unwrap().ends_with(&[0, 0, 0xff, 0xff]));
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
