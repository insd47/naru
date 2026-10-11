//! 클라이언트가 보내는 고정 형식의 제어 메시지입니다.
use super::encoding;
use crate::record::RecordKeys;
use aes::Aes128;
use aes::cipher::BlockEncrypt;
use aes::cipher::KeyInit;
use aes::cipher::generic_array::GenericArray;

/// ClientInit. 1바이트이며 공유 플래그 대신 서버가 정한 값을 보냅니다.
pub const CLIENT_INIT: [u8; 1] = [0xc1];

/// 암호화 시작 요청. ViewerInfo와 함께 보냅니다. 값을 0으로 바꾸면 핸드셰이크가 실패합니다.
pub const ENCRYPTION_START: [u8; 12] = [0x12, 0, 0, 1, 0, 1, 0, 1, 0, 0, 0, 1];

/// 암호화 활성화 요청. 키 교체를 받은 뒤 평문으로 보내며, 이후 보내는 모든 메시지는 레코드로 감쌉니다.
pub const ENCRYPTION_ACTIVATE: [u8; 8] = [0x12, 0, 0, 2, 0, 1, 0, 0];

/// 서버의 클립보드 감시를 켭니다. 세션마다 한 번 보냅니다.
pub const AUTO_CLIPBOARD: [u8; 8] = [0x15, 0, 0, 1, 0, 0, 0, 0];

// 서버는 기능 비트맵으로 주기적 상태 메시지와 클립보드 변경 알림 같은 기능을 켭니다.
// OS 버전은 실제 기기와 관계없이 15.3.0을 보냅니다. 서버가 이 값으로 처리 경로를 고릅니다.
const APP_ID: u32 = 2;
const APP_VERSION: [u32; 3] = [6, 1, 0];
const OS_VERSION: [u32; 3] = [15, 3, 0];
const CAPABILITIES: [(usize, u8); 5] = [(0, 0xb0), (2, 0x0c), (3, 0x03), (4, 0x90), (10, 0x40)];

/// 클라이언트의 종류와 지원 기능을 알리는 ViewerInfo(66바이트)를 만듭니다.
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

/// 지원하는 사각형 인코딩 목록(SetEncodings)을 만듭니다.
pub fn set_encodings() -> Vec<u8> {
    let mut message = vec![0x02, 0];
    message.extend_from_slice(&(encoding::SUPPORTED.len() as u16).to_be_bytes());

    for encoding in encoding::SUPPORTED {
        message.extend_from_slice(&encoding.to_be_bytes());
    }

    message
}

/// 프레임버퍼 갱신 요청을 만듭니다.
///
/// - 화면 픽셀은 UDP로 오므로, 이 요청은 서버가 TCP로 보내는 커서와 레이아웃 갱신을 깨워 두는 용도입니다.
/// - 큰 영역을 증분으로 계속 요청하면 서버가 TCP로 영상을 보내기 시작하므로, 주기 요청은 1×1로 보냅니다.
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

/// 키 누름 또는 뗌(KeyEvent)을 만듭니다. 키는 X11 키심입니다.
pub fn key(keysym: u32, down: bool) -> [u8; 8] {
    let [first, second, third, fourth] = keysym.to_be_bytes();
    [4, down.into(), 0, 0, first, second, third, fourth]
}

/// 암호화된 포인터 이벤트를 만듭니다. 이 메시지도 다시 레코드로 감싸 보냅니다.
///
/// - 좌표는 디코딩된 영상의 픽셀 좌표이며, 호출자가 영상 크기 안으로 자릅니다.
/// - 버튼 마스크는 왼쪽 1, 오른쪽 2, 가운데 4입니다. 표준 RFB와 오른쪽, 가운데 비트의 위치가 다릅니다.
/// - `time_ms`는 이벤트 시각입니다. 같은 위치·버튼의 이벤트가 이어져도 서버가 중복으로 버리지 않게 합니다. 단위는 확인하지 않았습니다.
pub fn pointer(keys: &RecordKeys, buttons: u8, x: u16, y: u16, time_ms: u32) -> [u8; 18] {
    let mut plain = [0; 16];
    plain[6..10].copy_from_slice(&time_ms.to_be_bytes());
    plain[10] = 0xff;
    plain[11] = buttons;
    plain[12..14].copy_from_slice(&x.to_be_bytes());
    plain[14..16].copy_from_slice(&y.to_be_bytes());

    let mut block = GenericArray::from(plain);
    Aes128::new(&keys.content_key.into()).encrypt_block(&mut block);

    let mut message = [0; 18];
    message[0] = 0x10;
    message[2..].copy_from_slice(&block);
    message
}

/// Mac의 입력 소스를 지정합니다(예: `com.apple.inputmethod.Korean.2SetKorean`).
///
/// - `1a 00, u16 길이, u16 버전(1), u16 ID 길이, ID`. 길이는 길이 필드 뒤의 바이트 수입니다.
/// - 받아들이면 서버가 새 입력 소스를 `0x455`로 알립니다. 유형 뒤의 패딩이 없으면 서버가 이후 메시지의 경계를 잃습니다.
/// - 한국어 입력 소스에서는 글자 키를 호환 자모 keysym으로 보내야 Mac이 조합합니다.
pub fn input_source(id: &str) -> Vec<u8> {
    let id = &id.as_bytes()[..id.len().min(u16::MAX as usize - 16)];
    let mut message = vec![0x1a, 0];
    message.extend_from_slice(&((4 + id.len()) as u16).to_be_bytes());
    message.extend_from_slice(&1u16.to_be_bytes());
    message.extend_from_slice(&(id.len() as u16).to_be_bytes());
    message.extend_from_slice(id);
    message
}

/// 서버의 클립보드를 요청합니다. 정확히 8바이트여야 하며, 1바이트라도 더 보내면 이후 메시지의 경계가 어긋납니다.
pub fn clipboard_request() -> [u8; 8] {
    [0x0b, 0, 0, 0, 0, 0, 0, 0]
}

#[cfg(test)]
mod tests {
    use super::{framebuffer_request, key, pointer, set_encodings, viewer_info};
    use crate::record::RecordKeys;
    use aes::Aes128;
    use aes::cipher::BlockDecrypt;
    use aes::cipher::KeyInit;
    use aes::cipher::generic_array::GenericArray;

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
}
