//! Control channel messages.
//!
//! - Client messages are built as bytes; server messages are split at their boundaries and then parsed rectangle by rectangle.
//! - Every length the server sends is checked against an upper bound before reading. Malformed input returns an error instead of panicking.
pub use cursor::{CursorImage, Cursors};
pub use display_config::{DisplayConfig, Mode};
pub use input_source::InputSource;
pub use layout::Layout;
pub use server::{Messages, Rect, ServerMessage};

pub mod client;
pub mod clipboard;
mod cursor;
mod display_config;
mod encoding;
pub mod gesture;
mod input_source;
mod layout;
mod server;
mod zlib;
