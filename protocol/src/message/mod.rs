//! 제어 채널의 메시지입니다.
//!
//! - 클라이언트 메시지는 바이트로 만들고, 서버 메시지는 경계를 나눈 뒤 사각형 단위로 해석합니다.
//! - 서버가 보낸 길이는 모두 상한을 확인한 뒤에 읽습니다. 형식이 맞지 않으면 패닉하지 않고 에러를 반환합니다.
pub use cursor::{CursorImage, Cursors};
pub use display::{DisplayConfig, MAX_PIXELS, Mode};
pub use layout::{Area, Display, Layout};
pub use server::{Messages, Rect, ServerMessage};

pub mod client;
pub mod clipboard;
mod cursor;
mod display;
pub mod encoding;
mod layout;
mod server;
mod zlib;
