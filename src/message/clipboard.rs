//! The clipboard transfer message (`0x1f`).
//!
//! - The content is a zlib-compressed list of items, each with a type identifier (UTI), aliases, and data.
//! - Content moves in two steps, as with Apple's client: an offer lists the kinds with no data, and the data follows
//!   only when the other side asks for it (`0x0b` from the client, status 3 from the Mac). The header's third byte tells
//!   them apart, and an offer the client asked for carries back the client's token.
//! - Text travels as a `public.utf8-plain-text` item, because standard RFB ClientCutText does not paste in current
//!   apps.
use super::zlib;
use crate::error::{Error, Result};
use crate::reader::Reader;
use flate2::{Compress, Compression, Decompress, FlushCompress};

const MAX_CONTENT_BYTES: usize = 64 << 20;
const MAX_ITEMS: u32 = 256;
const MAX_ALIASES: u32 = 64;

/// One representation of the clipboard's content.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ClipboardItem {
    /// Uniform type identifier, such as `public.png`
    pub kind: String,
    pub data: Vec<u8>,
}

/// What a clipboard message carries.
#[derive(Debug, PartialEq, Eq)]
pub enum Content {
    /// The kinds the clipboard holds, without data. `token` is the one the request carried, or 0.
    Offer { token: u32, kinds: Vec<String> },
    /// The clipboard's data, one item per kind
    Data(Vec<ClipboardItem>),
}

/// Reads a clipboard message. An empty clipboard is data with no items.
pub fn parse(message: &[u8]) -> Result<Content> {
    let mut reader = Reader::new(message, "clipboard");
    reader.skip(2)?;
    let offer = reader.u8()? == 1;
    reader.skip(1)?;
    let token = reader.u32()?;

    let len = reader.u32()? as usize;
    let compressed_len = reader.u32()? as usize;
    let compressed = reader.bytes(compressed_len)?;
    let items = if len == 0 { Vec::new() } else { items(compressed, len)? };

    Ok(if offer {
        Content::Offer {
            token,
            kinds: items.into_iter().map(|item| item.kind).collect(),
        }
    } else {
        Content::Data(items)
    })
}

fn items(compressed: &[u8], len: usize) -> Result<Vec<ClipboardItem>> {
    if len > MAX_CONTENT_BYTES {
        return Err(Error::TooLong { what: "clipboard", len });
    }

    let content = zlib::inflate(&mut Decompress::new(true), compressed, len, "clipboard")?;
    let mut reader = Reader::new(&content, "clipboard");
    let count = reader.u32()?;

    if count > MAX_ITEMS {
        return Err(Error::Malformed("clipboard"));
    }

    let mut items = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let kind = String::from_utf8_lossy(string(&mut reader)?).into_owned();
        reader.skip(4)?;

        let aliases = reader.u32()?;

        if aliases > MAX_ALIASES {
            return Err(Error::Malformed("clipboard"));
        }

        for _ in 0..aliases * 2 {
            string(&mut reader)?;
        }

        let data = string(&mut reader)?.to_vec();
        items.push(ClipboardItem { kind, data });
    }

    Ok(items)
}

/// Builds an offer of the client's clipboard, listing its kinds without data.
pub fn offer(kinds: &[String]) -> Result<Vec<u8>> {
    let items: Vec<_> = kinds
        .iter()
        .map(|kind| ClipboardItem {
            kind: kind.clone(),
            data: Vec::new(),
        })
        .collect();
    message(true, &items)
}

/// Builds the client's clipboard data, failing with `TooLong` past the same size limit used when receiving.
pub fn data(items: &[ClipboardItem]) -> Result<Vec<u8>> {
    message(false, items)
}

fn message(offer: bool, items: &[ClipboardItem]) -> Result<Vec<u8>> {
    let len = 4 + items
        .iter()
        .map(|item| item.kind.len() + item.data.len() + 16)
        .sum::<usize>();

    if len > MAX_CONTENT_BYTES {
        return Err(Error::TooLong { what: "clipboard", len });
    }

    // Items go without aliases, which the Mac derives from the type identifier.
    let mut content = Vec::with_capacity(len);
    content.extend_from_slice(&length(items.len()));

    for item in items {
        content.extend_from_slice(&length(item.kind.len()));
        content.extend_from_slice(item.kind.as_bytes());
        content.extend_from_slice(&[0; 8]);
        content.extend_from_slice(&length(item.data.len()));
        content.extend_from_slice(&item.data);
    }

    // End with a sync flush and no final block, like Apple's client. With a final block the Mac drops the content and
    // just clears the clipboard.
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

    let mut message = vec![0x1f, 0, offer.into(), 0, 0, 0, 0, 0];
    message.extend_from_slice(&length(content.len()));
    message.extend_from_slice(&length(compressed.len()));
    message.extend_from_slice(&compressed);
    Ok(message)
}

// Length fields are `u32`. The content was checked against [`MAX_CONTENT_BYTES`], and compression does not grow it past
// that.
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
    use super::{ClipboardItem, Content, data, offer, parse};

    const TEXT: &str = "public.utf8-plain-text";

    fn text(text: &str) -> ClipboardItem {
        ClipboardItem {
            kind: TEXT.into(),
            data: text.as_bytes().to_vec(),
        }
    }

    #[test]
    fn items_round_trip_through_a_message() {
        let image = ClipboardItem {
            kind: "public.png".into(),
            data: vec![0x89, b'P', b'N', b'G'],
        };
        let items = vec![text("안녕, Mac"), image];
        assert_eq!(parse(&data(&items).unwrap()).unwrap(), Content::Data(items));
    }

    // An offer lists kinds with empty data and is flagged in the header, as Apple's client sends it.
    #[test]
    fn offer_lists_kinds_without_data() {
        let kinds = vec!["public.png".to_owned(), TEXT.to_owned()];
        let message = offer(&kinds).unwrap();
        assert_eq!(message[..8], [0x1f, 0, 1, 0, 0, 0, 0, 0]);
        assert_eq!(parse(&message).unwrap(), Content::Offer { token: 0, kinds });
    }

    // The Mac answers an offer request with the token it carried.
    #[test]
    fn offer_from_the_mac_carries_the_token() {
        let mut message = offer(&["public.png".to_owned()]).unwrap();
        message[4..8].copy_from_slice(&0x6b8b_4568u32.to_be_bytes());
        assert!(matches!(
            parse(&message).unwrap(),
            Content::Offer { token: 0x6b8b_4568, .. }
        ));
    }

    // Apple's client also ends the compressed body with a sync flush (`00 00 ff ff`).
    #[test]
    fn content_ends_with_a_sync_flush() {
        assert!(data(&[text("안녕, Mac")]).unwrap().ends_with(&[0, 0, 0xff, 0xff]));
    }

    #[test]
    fn empty_clipboard_has_no_items() {
        let empty = [0x1f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(parse(&empty).unwrap(), Content::Data(Vec::new()));
    }

    #[test]
    fn declared_length_beyond_the_limit_is_refused() {
        let mut header = vec![0x1f, 0, 0, 0, 0, 0, 0, 0];
        header.extend_from_slice(&u32::MAX.to_be_bytes());
        header.extend_from_slice(&0u32.to_be_bytes());
        assert!(parse(&header).is_err());
    }
}
