//! macOS 화면 공유 High Performance 모드 클라이언트의 공통 코어입니다.
//!
//! - `session`이 소켓을 쥐고 접속부터 미디어 수신까지 처리하며, 네이티브 계층에는 완성된 접근 단위와 디코더 설정만 넘깁니다.
//! - 나머지 모듈은 I/O 없는 프로토콜 구현입니다. 서버가 보낸 길이는 모두 상한을 확인하며, 형식이 맞지 않으면 패닉하지 않고 에러를 반환합니다.
#![forbid(unsafe_code)]

pub use error::{Error, Result};
pub use session::{Config, Event, Session, Stage};

pub mod auth;
mod error;
pub mod media;
pub mod message;
pub mod offer;
mod reader;
pub mod record;
mod session;
