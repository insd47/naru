use super::encoding;
use crate::error::{Error, Result};
use crate::reader::Reader;

// 한 메시지의 크기 한도입니다. 가장 큰 메시지는 클립보드이며, 커서 이미지는 압축해서 수십 KiB입니다.
const MAX_CLIPBOARD_BYTES: usize = 32 << 20;
const MAX_CURSOR_BYTES: usize = 1 << 20;
const MAX_STATUS_BYTES: usize = 1 << 10;
const MAX_CUT_TEXT_BYTES: usize = 1 << 20;
const MAX_PENDING_BYTES: usize = MAX_CLIPBOARD_BYTES + (1 << 20);

/// 서버가 제어 채널로 보내는 메시지입니다.
#[derive(Debug, PartialEq)]
pub enum ServerMessage {
    /// 프레임버퍼 갱신. High Performance에서는 화면 픽셀 대신 커서, 레이아웃, 미디어 응답 같은 사각형을 싣습니다.
    FramebufferUpdate(Vec<Rect>),
    /// 상태 알림. 12는 약 2초 주기 심장박동, 2는 원격 클립보드 변경, 11은 사용자 세션 변경입니다.
    Status(u16),
    /// 클립보드 전송 메시지 전체. `clipboard::text`로 텍스트를 꺼냅니다.
    Clipboard(Vec<u8>),
    /// 표준 RFB의 벨과 ClientCutText 같은, 처리하지 않는 메시지
    Ignored(u8),
}

/// 프레임버퍼 갱신에 실린 사각형입니다.
#[derive(Debug, PartialEq)]
pub enum Rect {
    /// 레코드 계층의 키 교체. `RecordKeys::unwrap`으로 풉니다.
    KeyExchange([u8; 36]),
    /// 커서 모양. 사각형의 위치는 핫스팟, 크기는 커서 크기입니다.
    Cursor {
        hotspot: (u16, u16),
        size: (u16, u16),
        cache_id: u32,
        /// zlib으로 압축한 이미지. 비어 있으면 이미 받은 `cache_id`의 모양으로 바꾸라는 뜻입니다.
        data: Vec<u8>,
    },
    /// 디스플레이 레이아웃 본문. `Layout::parse`로 해석합니다.
    Layout(Vec<u8>),
    /// 미디어 스트림 사각형 본문. 미디어 오퍼의 응답이 실려 옵니다.
    MediaStream(Vec<u8>),
    /// Mac의 현재 입력 소스. `u16` 길이 뒤의 본문입니다.
    InputSource(Vec<u8>),
    /// 길이를 알고 건너뛴 사각형의 인코딩
    Skipped(i32),
    /// 길이를 알 수 없는 인코딩. 이 사각형부터 메시지 끝까지를 `rest`로 남기고 순회를 멈춥니다.
    Unknown { encoding: i32, rest: Vec<u8> },
}

impl ServerMessage {
    /// 버퍼의 앞에서 메시지 하나를 해석해, 메시지와 소비한 길이를 반환합니다. 메시지가 아직 다 오지 않았으면 `None`입니다.
    ///
    /// - 형식을 알 수 없는 메시지와 사각형은 경계를 알 수 없으므로 버퍼의 나머지를 모두 소비합니다.
    pub fn parse(buf: &[u8]) -> Result<Option<(Self, usize)>> {
        let mut reader = Reader::new(buf, "제어 메시지");

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
                    return Err(Error::Malformed("상태 알림"));
                };

                Self::Status(u16::from_be_bytes([high, low]))
            }
            0x1f => {
                let header = reader.bytes(15)?;
                let len = u32::from_be_bytes(header[11..15].try_into().expect("4바이트다")) as usize;
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

/// 받은 바이트를 메시지 단위로 꺼냅니다.
///
/// - 평문 구간에서는 TCP로 받은 바이트를, 레코드 구간에서는 레코드 본문을 차례로 넣습니다. 큰 메시지는 이어지는 레코드에 유형 바이트 없이 나뉘어 오므로, 본문을 이어 붙이면 됩니다.
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
                    what: "제어 메시지",
                    len: self.buffer.len(),
                });
            }

            return Ok(None);
        };

        self.buffer.drain(..consumed);
        Ok(Some(message))
    }

    /// 아직 꺼내지 않은 바이트를 넘겨줍니다. 평문 구간이 끝난 뒤 남은 바이트는 레코드입니다.
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

    // 키 교체 앞에 미디어 스트림 사각형이 오고, 뒤에 상태 알림과 레코드가 이어 붙은 수신 묶음입니다.
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

        // 이어지는 레코드에는 유형 바이트가 없습니다.
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
