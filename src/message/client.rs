//! Fixed-format control messages the client sends.
use super::encoding;
use crate::record::RecordKeys;
use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockEncrypt, KeyInit};

/// ClientInit, a single byte carrying the value the server expects instead of a shared flag.
pub const CLIENT_INIT: [u8; 1] = [0xc1];

/// Encryption start request, sent along with ViewerInfo. Zeroing these values makes the handshake fail.
pub const ENCRYPTION_START: [u8; 12] = [0x12, 0, 0, 1, 0, 1, 0, 1, 0, 0, 0, 1];

/// Encryption activation request, sent in plaintext after the key exchange. Every later message is wrapped in a record.
pub const ENCRYPTION_ACTIVATE: [u8; 8] = [0x12, 0, 0, 2, 0, 1, 0, 0];

/// Turns on the server's clipboard monitoring. Sent once per session.
pub const AUTO_CLIPBOARD: [u8; 8] = [0x15, 0, 0, 1, 0, 0, 0, 0];

// The capability bitmap makes the server enable features such as periodic status messages and clipboard change notices.
// The OS version is always 15.3.0 regardless of the device, because the server picks its code path from it.
const APP_ID: u32 = 2;
const APP_VERSION: [u32; 3] = [6, 1, 0];
const OS_VERSION: [u32; 3] = [15, 3, 0];
const CAPABILITIES: [(usize, u8); 5] = [(0, 0xb0), (2, 0x0c), (3, 0x03), (4, 0x90), (10, 0x40)];

/// Builds the 66-byte ViewerInfo that announces the client kind and its capabilities.
pub fn viewer_info() -> Vec<u8> {
    let mut message = vec![0x21, 0];
    message.extend_from_slice(&62u16.to_be_bytes());
    message.extend_from_slice(&1u16.to_be_bytes());
    message.extend_from_slice(&APP_ID.to_be_bytes());

    for part in APP_VERSION.iter().chain(&OS_VERSION) {
        message.extend_from_slice(&part.to_be_bytes());
    }

    let mut capabilities = [0; 32];

    for (at, bits) in CAPABILITIES {
        capabilities[at] = bits;
    }

    message.extend_from_slice(&capabilities);
    message
}

/// Builds the list of supported rectangle encodings (SetEncodings).
pub fn set_encodings() -> Vec<u8> {
    let mut message = vec![0x02, 0];
    message.extend_from_slice(&(encoding::SUPPORTED.len() as u16).to_be_bytes());

    for encoding in encoding::SUPPORTED {
        message.extend_from_slice(&encoding.to_be_bytes());
    }

    message
}

/// Builds a framebuffer update request.
///
/// - Screen pixels arrive over UDP, so this request only keeps the server sending cursor and layout updates over TCP.
/// - Repeated incremental requests for a large area make the server start sending video over TCP, so periodic requests
///   are 1×1.
pub fn framebuffer_request(incremental: bool, width: u16, height: u16) -> [u8; 10] {
    let [width_high, width_low] = width.to_be_bytes();
    let [height_high, height_low] = height.to_be_bytes();
    [
        3,
        incremental.into(),
        0,
        0,
        0,
        0,
        width_high,
        width_low,
        height_high,
        height_low,
    ]
}

/// Builds a key press or release (KeyEvent). The key is an X11 keysym.
pub fn key(keysym: u32, down: bool) -> [u8; 8] {
    let [first, second, third, fourth] = keysym.to_be_bytes();
    [4, down.into(), 0, 0, first, second, third, fourth]
}

/// Builds SetKeyboardInputSource, which selects a Mac input source by its ID. The agent selects it at the next key.
///
/// - The type is followed by one byte of padding; without it the server loses message boundaries.
pub fn input_source(id: &str) -> Vec<u8> {
    let id = &id.as_bytes()[..id.len().min(usize::from(u16::MAX) - 4)];
    let mut message = vec![0x1a, 0];
    message.extend_from_slice(&((4 + id.len()) as u16).to_be_bytes());
    message.extend_from_slice(&1u16.to_be_bytes());
    message.extend_from_slice(&(id.len() as u16).to_be_bytes());
    message.extend_from_slice(id);
    message
}

/// Builds an encrypted pointer event. This message is still wrapped in a record when sent.
///
/// - Coordinates are pixels of the decoded video; the caller clamps them to the video size.
/// - The button mask is left 1, right 2, middle 4. The right and middle bits are swapped relative to standard RFB.
/// - `elapsed_us` is the microseconds since the previous pointer event, summing to real elapsed time.
///   The Mac builds event timestamps from it, so it also decides double-click intervals.
pub fn pointer(keys: &RecordKeys, buttons: u8, x: u16, y: u16, elapsed_us: u32) -> [u8; 18] {
    let mut plain = [0; 16];
    plain[6..10].copy_from_slice(&elapsed_us.to_be_bytes());
    plain[10] = 0xff;
    plain[11] = buttons;
    plain[12..14].copy_from_slice(&x.to_be_bytes());
    plain[14..16].copy_from_slice(&y.to_be_bytes());
    encrypted(keys, plain)
}

/// Builds an encrypted key event with the physical key, as Apple's client sends every key. This message is still
/// wrapped in a record when sent.
///
/// - `keycode` is the macOS virtual key code of the physical key (`kVK_ANSI_F` for both `f` and `ㄹ`). The agent
///   presses that key, so the Mac's input method composes it even when the agent still holds another source's layout,
///   which a keysym alone (`key`) is looked up in.
/// - `elapsed_us` is the microseconds since the previous key event.
/// - The keyboard type is an ANSI Mac keyboard's, as Apple's client sent from a MacBook.
pub fn key_event(keys: &RecordKeys, keysym: u32, down: bool, keycode: u16, elapsed_us: u32) -> [u8; 18] {
    let mut plain = [0; 16];
    plain[0] = 0xff;
    plain[1] = down.into();
    plain[2..6].copy_from_slice(&keysym.to_be_bytes());
    plain[6..10].copy_from_slice(&elapsed_us.to_be_bytes());
    plain[10..14].copy_from_slice(&ANSI_KEYBOARD.to_be_bytes());
    plain[14..16].copy_from_slice(&keycode.to_be_bytes());
    encrypted(keys, plain)
}

// Keyboard type 40, which the agent also logs as the viewer's
const ANSI_KEYBOARD: u32 = 40;

// An input event: its 16 bytes encrypted with the content key as one AES block, after the type and a padding byte
fn encrypted(keys: &RecordKeys, plain: [u8; 16]) -> [u8; 18] {
    let mut block = GenericArray::from(plain);
    Aes128::new(&keys.content_key.into()).encrypt_block(&mut block);

    let mut message = [0; 18];
    message[0] = 0x10;
    message[2..].copy_from_slice(&block);
    message
}

/// Requests the kinds on the server's clipboard. The offer that answers carries `token` back. It must be exactly 8
/// bytes; any extra byte misaligns every later message.
pub fn clipboard_offer_request(token: u32) -> [u8; 8] {
    let [first, second, third, fourth] = token.to_be_bytes();
    [0x0b, 1, 0, 0, first, second, third, fourth]
}

/// Requests the data on the server's clipboard.
pub fn clipboard_request() -> [u8; 8] {
    [0x0b, 0, 0, 0, 0, 0, 0, 0]
}

#[cfg(test)]
mod tests {
    use super::{
        clipboard_offer_request, clipboard_request, framebuffer_request, key, key_event, pointer, set_encodings,
        viewer_info,
    };
    use crate::record::RecordKeys;
    use aes::Aes128;
    use aes::cipher::generic_array::GenericArray;
    use aes::cipher::{BlockDecrypt, KeyInit};

    #[test]
    fn viewer_info_matches_the_captured_bytes() {
        let expected = [
            "2100003e0001000000020000000600000001000000000000000f0000000300000000b000",
            "0c0390000000000040000000000000000000000000000000000000000000",
        ]
        .concat();

        let hex: String = viewer_info().iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(hex, expected);
        assert_eq!(viewer_info().len(), 66);
    }

    #[test]
    fn set_encodings_lists_thirteen_encodings_with_desktop_size_negative() {
        let message = set_encodings();
        assert_eq!(&message[..4], [2, 0, 0, 13]);
        assert_eq!(message.len(), 4 + 13 * 4);
        assert_eq!(&message[32..36], (-223i32).to_be_bytes());
    }

    #[test]
    fn periodic_request_is_incremental_one_by_one() {
        assert_eq!(framebuffer_request(true, 1, 1), [3, 1, 0, 0, 0, 0, 0, 1, 0, 1]);
        assert_eq!(
            framebuffer_request(false, 0xffff, 0xffff),
            [3, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]
        );
    }

    #[test]
    fn key_event_carries_the_keysym_big_endian() {
        assert_eq!(key(0xffeb, true), [4, 1, 0, 0, 0, 0, 0xff, 0xeb]);
    }

    // Byte for byte the plaintext Apple's client sent for Shift down in the 2026-10-19 relay capture.
    #[test]
    fn key_event_plaintext_matches_apple() {
        let keys = RecordKeys {
            content_key: [5; 16],
            iv: [0; 16],
        };

        let message = key_event(&keys, 0xffe1, true, 0x38, 0x0007_5e8d);
        assert_eq!(&message[..2], [0x10, 0]);

        let mut block = GenericArray::clone_from_slice(&message[2..]);
        Aes128::new(&keys.content_key.into()).decrypt_block(&mut block);
        assert_eq!(
            block.as_slice(),
            [0xff, 1, 0, 0, 0xff, 0xe1, 0, 7, 0x5e, 0x8d, 0, 0, 0, 0x28, 0, 0x38]
        );
    }

    #[test]
    fn pointer_plaintext_has_time_then_marker_buttons_and_coordinates() {
        let keys = RecordKeys {
            content_key: [5; 16],
            iv: [0; 16],
        };

        let message = pointer(&keys, 0b10, 1919, 1079, 0x0102_0304);
        assert_eq!(&message[..2], [0x10, 0]);

        let mut block = GenericArray::clone_from_slice(&message[2..]);
        Aes128::new(&keys.content_key.into()).decrypt_block(&mut block);
        assert_eq!(
            block.as_slice(),
            [0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 0xff, 2, 0x07, 0x7f, 0x04, 0x37]
        );
    }

    #[test]
    fn input_source_has_padding_and_lengths() {
        let message = super::input_source("com.apple.keylayout.ABC");
        let mut expected = vec![0x1a, 0, 0, 27, 0, 1, 0, 23];
        expected.extend_from_slice(b"com.apple.keylayout.ABC");
        assert_eq!(message, expected);
    }

    // Byte for byte what Apple's client sent in the 2026-10-24 relay capture: an offer request with its token, then a
    // data request.
    #[test]
    fn clipboard_requests_match_apple() {
        assert_eq!(
            clipboard_offer_request(0x643c_986a),
            [0x0b, 1, 0, 0, 0x64, 0x3c, 0x98, 0x6a]
        );
        assert_eq!(clipboard_request(), [0x0b, 0, 0, 0, 0, 0, 0, 0]);
    }
}
