//! Protocol core of a client for macOS Screen Sharing's High Performance mode.
//!
//! - `session` owns the sockets and handles everything from connecting to receiving media; the native layer only gets complete access units and decoder configs.
//! - The other modules are I/O-free protocol code. Every length the server sends is checked against a limit, and malformed input returns an error instead of panicking.
//! - `unsafe` is allowed only at the FFI boundary that UniFFI generates (`ffi`).
//! - `record`, `RtpPacket`, and `SrtpContext` are primitives that integration tests acting as the server use to build records and media.
//! - `auth` exposes the RFB version string so the probe can test the handshake without authenticating.
#![deny(unsafe_code)]

pub use error::{Error, Result};
pub use media::{AccessUnit, AudioFrame, DecoderConfig, Priority, RtpPacket, SrtpContext, VideoCodec};
pub use message::gesture::Phase;
pub use message::{CursorImage, DisplayConfig, InputSource, Layout, Mode};
pub use session::{Config, Event, MediaStats, Session, Stage, TouchPhase};

pub mod auth;
mod error;
#[allow(unsafe_code)]
mod ffi;
mod keyboard;
mod media;
mod message;
mod offer;
mod reader;
pub mod record;
mod session;

uniffi::setup_scaffolding!();
