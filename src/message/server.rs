use super::encoding;
use crate::error::{Error, Result};
use crate::reader::Reader;

// Per-message size limits. The clipboard is the largest message; a compressed cursor image is tens of KiB.
const MAX_CLIPBOARD_BYTES: usize = 32 << 20;
const MAX_CURSOR_BYTES: usize = 1 << 20;
const MAX_STATUS_BYTES: usize = 1 << 10;
const MAX_CUT_TEXT_BYTES: usize = 1 << 20;
const MAX_PENDING_BYTES: usize = MAX_CLIPBOARD_BYTES + (1 << 20);

const STATUS_CLIPBOARD_CHANGED: u16 = 2;
const STATUS_CURSOR_HIDDEN: u16 = 11;

/// A message the server sends on the control channel.
#[derive(Debug, PartialEq)]
pub enum ServerMessage {
    /// Framebuffer update. In High Performance mode it carries rectangles such as cursor, layout, and media answers
    /// instead of screen pixels.
    FramebufferUpdate(Vec<Rect>),
    /// Notice that the Mac's clipboard changed (status code 2). Request the content with `client::clipboard_request`.
    ClipboardChanged,
    /// The Mac hid its cursor, as it does while typing or over playing video (status code 11). The next cursor shape
    /// shows it again.
    CursorHidden,
    /// Any other status notice, such as the periodic heartbeat (12)
    Status(u16),
    /// A whole clipboard transfer message. Extract the text with `clipboard::text`.
    Clipboard(Vec<u8>),
    /// A message that is not handled, such as standard RFB Bell and ServerCutText
    Ignored(u8),
}

/// A rectangle carried in a framebuffer update.
#[derive(Debug, PartialEq)]
pub enum Rect {
    /// Record layer key exchange. Unwrap it with `RecordKeys::unwrap`.
    KeyExchange([u8; 36]),
    /// Cursor shape. The rectangle's position is the hotspot and its size is the cursor size.
    Cursor {
        hotspot: (u16, u16),
        size: (u16, u16),
        cache_id: u32,
        /// zlib-compressed image. Empty means switch to the already received shape for `cache_id`.
        data: Vec<u8>,
    },
    /// Display layout body. Parse it with `Layout::parse`.
    Layout(Vec<u8>),
    /// Media stream rectangle body, which carries the answer to the media offer
    MediaStream(Vec<u8>),
    /// The Mac's current input source, as the body after the `u16` length
    InputSource(Vec<u8>),
    /// Encoding of a rectangle with a known length that was skipped
    Skipped(i32),
    /// Encoding with an unknown length. Everything from this rectangle to the end of the message is kept in `rest`, and
    /// the walk stops.
    Unknown { encoding: i32, rest: Vec<u8> },
}

impl ServerMessage {
    /// Parses one message from the front of the buffer, returning it with the consumed length, or `None` when it has
    /// not fully arrived.
    ///
    /// - An unknown message or rectangle has no known boundary, so it consumes the rest of the buffer.
    pub fn parse(buf: &[u8]) -> Result<Option<(Self, usize)>> {
        let mut reader = Reader::new(buf, "control message");

        match Self::read(&mut reader) {
            Ok(message) => Ok(Some((message, buf.len() - reader.remaining()))),
            Err(Error::Truncated(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn read(reader: &mut Reader) -> Result<Self> {
        let kind = reader.u8()?;

        Ok(match kind {
            0x00 => {
                reader.skip(1)?;
                let count = reader.u16()?;
                let mut rects = Vec::new();

                for _ in 0..count {
                    let rect = Rect::read(reader)?;
                    let stop = matches!(rect, Rect::Unknown { .. });
                    rects.push(rect);

                    if stop {
                        break;
                    }
                }

                Self::FramebufferUpdate(rects)
            }
            0x14 => {
                reader.skip(1)?;
                let len = reader.u16()? as usize;
                let body = reader.prefixed(len, MAX_STATUS_BYTES)?;
                let Some(&[high, low]) = body.get(2..4) else {
                    return Err(Error::Malformed("status notice"));
                };

                match u16::from_be_bytes([high, low]) {
                    STATUS_CLIPBOARD_CHANGED => Self::ClipboardChanged,
                    STATUS_CURSOR_HIDDEN => Self::CursorHidden,
                    code => Self::Status(code),
                }
            }
            0x1f => {
                let header = reader.bytes(15)?;
                let len = u32::from_be_bytes(header[11..15].try_into().expect("4 bytes")) as usize;
                let data = reader.prefixed(len, MAX_CLIPBOARD_BYTES)?;
                Self::Clipboard([&[kind], header, data].concat())
            }
            0x02 => Self::Ignored(kind),
            0x03 => {
                reader.skip(3)?;
                let len = reader.u32()? as usize;
                reader.prefixed(len, MAX_CUT_TEXT_BYTES)?;
                Self::Ignored(kind)
            }
            _ => {
                reader.rest();
                Self::Ignored(kind)
            }
        })
    }
}

impl Rect {
    fn read(reader: &mut Reader) -> Result<Self> {
        let x = reader.u16()?;
        let y = reader.u16()?;
        let width = reader.u16()?;
        let height = reader.u16()?;
        let encoding = reader.i32()?;

        let prefixed = |reader: &mut Reader| -> Result<Vec<u8>> {
            let len = reader.u16()? as usize;
            Ok(reader.bytes(len)?.to_vec())
        };

        Ok(match encoding {
            encoding::KEY_EXCHANGE => Self::KeyExchange(reader.array()?),
            encoding::CURSOR => {
                let cache_id = reader.u32()?;
                let len = reader.u32()? as usize;

                Self::Cursor {
                    hotspot: (x, y),
                    size: (width, height),
                    cache_id,
                    data: reader.prefixed(len, MAX_CURSOR_BYTES)?.to_vec(),
                }
            }
            encoding::DISPLAY_LAYOUT => Self::Layout(prefixed(reader)?),
            encoding::MEDIA_STREAM => Self::MediaStream(prefixed(reader)?),
            encoding::INPUT_SOURCE => Self::InputSource(prefixed(reader)?),
            encoding::STILL_IMAGE | encoding::STILL_IMAGE_LEGACY | encoding::VENDOR_KEYSYMS | encoding::DEVICE_INFO => {
                prefixed(reader)?;
                Self::Skipped(encoding)
            }
            encoding::DESKTOP_SIZE => Self::Skipped(encoding),
            _ => Self::Unknown {
                encoding,
                rest: reader.rest().to_vec(),
            },
        })
    }
}

/// Splits received bytes into messages.
///
/// - Push raw TCP bytes during the plaintext stretch and record bodies after it. A large message spans the following
///   records without a type byte, so concatenating bodies is enough.
#[derive(Default)]
pub struct Messages {
    buffer: Vec<u8>,
}

impl Messages {
    pub fn push(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
    }

    pub fn pop(&mut self) -> Result<Option<ServerMessage>> {
        if self.buffer.is_empty() {
            return Ok(None);
        }

        let Some((message, consumed)) = ServerMessage::parse(&self.buffer)? else {
            if self.buffer.len() > MAX_PENDING_BYTES {
                return Err(Error::TooLong {
                    what: "control message",
                    len: self.buffer.len(),
                });
            }

            return Ok(None);
        };

        self.buffer.drain(..consumed);
        Ok(Some(message))
    }

    /// Hands over the bytes not yet popped. Whatever remains after the plaintext stretch is records.
    pub fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::{Messages, Rect, ServerMessage};
    use crate::message::encoding;

    fn rect(encoding: i32, header: [u16; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes: Vec<u8> = header.iter().flat_map(|value| value.to_be_bytes()).collect();
        bytes.extend_from_slice(&encoding.to_be_bytes());
        bytes.extend_from_slice(body);
        bytes
    }

    fn prefixed(body: &[u8]) -> Vec<u8> {
        [&(body.len() as u16).to_be_bytes()[..], body].concat()
    }

    fn update(rects: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = vec![0, 0];
        bytes.extend_from_slice(&(rects.len() as u16).to_be_bytes());
        bytes.extend(rects.concat());
        bytes
    }

    fn drain(messages: &mut Messages) -> Vec<ServerMessage> {
        std::iter::from_fn(|| messages.pop().unwrap()).collect()
    }

    // A received batch: a status notice, then a media stream rectangle before the key exchange, then records.
    fn prelude() -> (Vec<u8>, Vec<u8>) {
        let key = [0x5a; 36];

        let mut bytes = vec![0x14, 0, 0, 4, 0, 1, 0, 12];
        bytes.extend(update(&[
            rect(encoding::MEDIA_STREAM, [0; 4], &prefixed(b"stream")),
            rect(encoding::KEY_EXCHANGE, [0; 4], &key),
        ]));

        let records = vec![0, 32, 1, 2, 3];
        bytes.extend_from_slice(&records);
        (bytes, records)
    }

    #[test]
    fn key_exchange_ends_exactly_where_records_begin() {
        let (bytes, records) = prelude();
        let mut messages = Messages::default();
        messages.push(&bytes);

        assert_eq!(messages.pop().unwrap(), Some(ServerMessage::Status(12)));

        let Some(ServerMessage::FramebufferUpdate(rects)) = messages.pop().unwrap() else {
            panic!()
        };

        assert_eq!(rects[0], Rect::MediaStream(b"stream".to_vec()));
        assert_eq!(rects[1], Rect::KeyExchange([0x5a; 36]));
        assert_eq!(messages.take(), records);
    }

    // Status notices measured on a Mac playing video full screen: 11 when the cursor hid, 12 as the heartbeat.
    #[test]
    fn status_notices_tell_a_hidden_cursor_from_the_rest() {
        let mut messages = Messages::default();
        messages.push(&[0x14, 0, 0, 4, 0, 1, 0, 11, 0x14, 0, 0, 4, 0, 1, 0, 4]);

        assert_eq!(messages.pop().unwrap(), Some(ServerMessage::CursorHidden));
        assert_eq!(messages.pop().unwrap(), Some(ServerMessage::Status(4)));
    }

    #[test]
    fn every_split_point_yields_the_same_messages() {
        let (bytes, records) = prelude();
        let bytes = &bytes[..bytes.len() - records.len()];

        let mut whole = Messages::default();
        whole.push(bytes);
        let expected = drain(&mut whole);

        for cut in 0..=bytes.len() {
            let mut messages = Messages::default();
            messages.push(&bytes[..cut]);
            let mut parsed = drain(&mut messages);
            messages.push(&bytes[cut..]);
            parsed.extend(drain(&mut messages));
            assert_eq!(parsed, expected, "cut at {cut}");
        }
    }

    #[test]
    fn cursor_store_and_select_rects_are_framed() {
        let mut store = 1000u32.to_be_bytes().to_vec();
        store.extend_from_slice(&3u32.to_be_bytes());
        store.extend_from_slice(b"zip");

        let mut select = 1000u32.to_be_bytes().to_vec();
        select.extend_from_slice(&0u32.to_be_bytes());

        let bytes = update(&[
            rect(encoding::CURSOR, [4, 5, 32, 32], &store),
            rect(encoding::CURSOR, [0; 4], &select),
        ]);

        let Some((ServerMessage::FramebufferUpdate(rects), consumed)) = ServerMessage::parse(&bytes).unwrap() else {
            panic!()
        };

        assert_eq!(consumed, bytes.len());

        assert_eq!(
            rects[0],
            Rect::Cursor {
                hotspot: (4, 5),
                size: (32, 32),
                cache_id: 1000,
                data: b"zip".to_vec(),
            }
        );

        assert!(matches!(&rects[1], Rect::Cursor { data, .. } if data.is_empty()));
    }

    #[test]
    fn unknown_rect_keeps_the_rest_and_stops_walking() {
        let bytes = update(&[
            rect(encoding::DESKTOP_SIZE, [0; 4], &[]),
            rect(0x7777, [0; 4], b"opaque"),
            rect(encoding::KEY_EXCHANGE, [0; 4], &[0; 36]),
        ]);

        let Some((ServerMessage::FramebufferUpdate(rects), consumed)) = ServerMessage::parse(&bytes).unwrap() else {
            panic!()
        };

        assert_eq!(consumed, bytes.len());
        assert_eq!(rects.len(), 2);
        assert!(matches!(&rects[1], Rect::Unknown { encoding: 0x7777, rest } if rest.starts_with(b"opaque")));
    }

    #[test]
    fn clipboard_spanning_records_is_reassembled() {
        let mut message = vec![0x1f, 0, 0, 0, 0, 0, 0, 0];
        message.extend_from_slice(&100u32.to_be_bytes());
        message.extend_from_slice(&40u32.to_be_bytes());
        message.extend_from_slice(&[9; 40]);

        // The continuing record has no type byte.
        let mut messages = Messages::default();
        messages.push(&message[..20]);
        assert_eq!(messages.pop().unwrap(), None);
        messages.push(&message[20..]);
        assert_eq!(messages.pop().unwrap(), Some(ServerMessage::Clipboard(message)));
    }

    #[test]
    fn oversized_clipboard_is_refused_before_buffering() {
        let mut header = vec![0x1f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        header.extend_from_slice(&u32::MAX.to_be_bytes());
        assert!(ServerMessage::parse(&header).is_err());
    }
}
