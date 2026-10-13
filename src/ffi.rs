//! The UniFFI boundary that exposes sessions to Kotlin and Swift.
//!
//! - The native side opens a session by passing a [`SessionListener`] to [`Session::connect`] and receives events in order on a dedicated session thread.
//! - Core types without tuples or private fields pass through as is; the rest are reshaped here for the native side.
use crate::{
    AccessUnit, AudioFrame, DecoderConfig, Error, InputSource, MediaStats, Mode, Phase, Priority, Stage, TouchPhase,
};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, LazyLock};
use tokio::runtime::Runtime;

// Sessions run on this crate's tokio runtime, so the native side needs no runtime of its own.
// Each session is a single task, so two worker threads are enough.
static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("naru-protocol")
        .enable_all()
        .build()
        .expect("cannot build the tokio runtime")
});

/// Connection settings
#[derive(uniffi::Record)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// Server public key (DER) saved from a previous connection. A mismatch stops the connection.
    pub known_key: Option<Vec<u8>>,
    /// Virtual display. Without one, the Mac's physical screen is streamed.
    pub display: Option<DisplayMode>,
    /// Whether to receive audio. Turn it off on devices without an AAC-ELD decoder.
    pub audio: bool,
    /// Whether latency or quality wins when packets are lost
    pub priority: Priority,
    /// Whether to receive HEVC 4:4:4. Turn it on only when the device's hardware decoder supports 4:4:4; off means H.264 4:2:0.
    pub hevc: bool,
}

impl From<Config> for crate::Config {
    // Every field is listed so that a new core config field causes a compile error here.
    fn from(config: Config) -> Self {
        Self {
            host: config.host,
            port: config.port,
            username: config.username,
            password: config.password,
            known_key: config.known_key,
            display: config.display.map(Into::into),
            audio: config.audio,
            priority: config.priority,
            codec: if config.hevc {
                crate::VideoCodec::Hevc
            } else {
                crate::VideoCodec::H264
            },
            local_ports: None,
        }
    }
}

/// Logical size and scale of the virtual display. The pixel size is logical size × scale.
///
/// - A pixel size past the server's limit (3840×2160 landscape, 2160×3840 portrait) is shrunk with the aspect ratio kept. The actual size arrives as `Layout`.
#[derive(uniffi::Record)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

impl From<DisplayMode> for crate::DisplayConfig {
    fn from(mode: DisplayMode) -> Self {
        Mode::fitting((mode.width, mode.height), mode.scale).into()
    }
}

/// An event the session reports
#[derive(uniffi::Enum)]
pub enum Event {
    Progress {
        stage: Stage,
    },
    /// Server public key (DER). On a first connection, save it and pass it as `known_key` next time.
    ServerKey {
        der: Vec<u8>,
    },
    Authenticated {
        server_proof: bool,
    },
    /// The Mac's computer name. Arrives once after authentication.
    ServerName {
        name: String,
    },
    /// Display layout. Pointer coordinates are sent relative to the pixel size.
    Layout {
        logical_width: u16,
        logical_height: u16,
        pixel_width: u16,
        pixel_height: u16,
    },
    // A variant named after the type it carries makes the generated Kotlin nested class shadow that type, so these two are named differently.
    /// New decoder config. Recreate the decoder with it.
    Decoder {
        config: DecoderConfig,
    },
    /// Audio decoder config (AudioSpecificConfig). Arrives before the first audio of each audio stream; recreate the audio decoder with it.
    AudioConfig {
        config: Vec<u8>,
    },
    /// Audio access unit
    Audio {
        frame: AudioFrame,
    },
    /// Access unit for the decoder
    Frame {
        unit: AccessUnit,
    },
    /// Cursor shape as row-major RGBA with non-premultiplied alpha. The position arrives as `Pointer` only when finger touches moved it.
    Cursor {
        width: u16,
        height: u16,
        hotspot_x: u16,
        hotspot_y: u16,
        rgba: Vec<u8>,
    },
    /// Cursor position moved by touch, in layout pixels. Draw the cursor here.
    Pointer {
        x: u16,
        y: u16,
    },
    /// The Mac's current input source. The session does character conversion itself, so this is only for logging and display.
    Source {
        source: InputSource,
    },
    Clipboard {
        text: String,
    },
    /// No frame has come out for too long. Recreate the decoder; the session requests the keyframe.
    ResetDecoder,
    /// Media reception stats. Arrives every 0.5 seconds after media negotiation.
    Stats {
        stats: MediaStats,
    },
    /// The session ended. Both fields are absent when it was closed.
    Closed {
        failure: Option<Failure>,
        /// The cause chain joined on one line, for logs
        detail: Option<String>,
    },
}

impl From<crate::Event> for Event {
    fn from(event: crate::Event) -> Self {
        match event {
            crate::Event::Progress(stage) => Self::Progress { stage },
            crate::Event::ServerKey(der) => Self::ServerKey { der },
            crate::Event::Authenticated { server_proof } => Self::Authenticated { server_proof },
            crate::Event::ServerName(name) => Self::ServerName { name },
            crate::Event::Layout(layout) => Self::Layout {
                logical_width: layout.logical.0,
                logical_height: layout.logical.1,
                pixel_width: layout.pixels.0,
                pixel_height: layout.pixels.1,
            },
            crate::Event::DecoderConfig(config) => Self::Decoder { config },
            crate::Event::AudioConfig(config) => Self::AudioConfig { config },
            crate::Event::Audio(frame) => Self::Audio { frame },
            crate::Event::Frame(unit) => Self::Frame { unit },
            crate::Event::Pointer { x, y } => Self::Pointer { x, y },
            crate::Event::Cursor(image) => Self::Cursor {
                width: image.width,
                height: image.height,
                hotspot_x: image.hotspot.0,
                hotspot_y: image.hotspot.1,
                rgba: image.rgba.clone(),
            },
            crate::Event::InputSource(source) => Self::Source { source },
            crate::Event::Clipboard(text) => Self::Clipboard { text },
            crate::Event::ResetDecoder => Self::ResetDecoder,
            crate::Event::Stats(stats) => Self::Stats { stats },
            crate::Event::Closed(error) => Self::Closed {
                failure: error.as_ref().map(Failure::from),
                detail: error.as_ref().map(|error| {
                    std::iter::successors(Some(error as &dyn std::error::Error), |error| error.source())
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(": ")
                }),
            },
        }
    }
}

/// Why the connection ended, used to pick the message shown to the user and the next action.
#[derive(uniffi::Enum)]
pub enum Failure {
    /// The username or password is wrong.
    Rejected,
    /// The server's public key differs from the saved key.
    KeyChanged,
    /// Another user is logged in on the Mac.
    OtherUser { name: String },
    /// The address could not be resolved or connected to.
    Unreachable,
    /// The server does not support this client's authentication method.
    Unsupported,
    /// The server never started sending video.
    NoMedia,
    /// Any other error
    Other,
}

impl From<&Error> for Failure {
    fn from(error: &Error) -> Self {
        let cause = match error {
            Error::Failed { source, .. } => source,
            error => error,
        };

        match cause {
            Error::Rejected => Self::Rejected,
            Error::KeyChanged => Self::KeyChanged,
            Error::OtherUser(name) => Self::OtherUser { name: name.clone() },
            Error::Io(_) | Error::Resolve(_) | Error::Timeout => Self::Unreachable,
            Error::Unsupported => Self::Unsupported,
            Error::NoMedia { .. } => Self::NoMedia,
            _ => Self::Other,
        }
    }
}

/// Receives a session's events.
#[uniffi::export(with_foreign)]
pub trait SessionListener: Send + Sync {
    /// One event. Called in order on the dedicated session thread; the last event is always `Closed`.
    ///
    /// - The next event waits until this returns, and the session waits when events back up. Hand long work to another thread.
    fn on_event(&self, event: Event);
}

/// A running session. Methods called after it ends do nothing.
#[derive(uniffi::Object)]
pub struct Session {
    session: crate::Session,
}

#[uniffi::export]
impl Session {
    /// Starts connecting and returns immediately. Progress arrives at `listener`.
    #[uniffi::constructor]
    pub fn connect(config: Config, listener: Arc<dyn SessionListener>) -> Arc<Self> {
        let _runtime = RUNTIME.enter();
        let (session, mut events) = crate::Session::start(config.into());

        // Each event calls native code, so a dedicated thread delivers them instead of holding a tokio worker.
        std::thread::Builder::new()
            .name("naru-events".into())
            .spawn(move || {
                // UniFFI turns a native exception into a panic. Drop only that event and keep delivering so the final `Closed` arrives.
                while let Some(event) = events.blocking_recv() {
                    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| listener.on_event(event.into())));
                }
            })
            .expect("cannot spawn the event thread");

        Arc::new(Self { session })
    }

    /// Key input. `keysym` is an X11 keysym; send character keys as the Latin letter of the physical key. The session handles conversion to Korean jamo.
    pub fn key(&self, keysym: u32, down: bool) {
        let _ = self.session.key(keysym, down);
    }

    /// Pointer position in layout pixels and pressed buttons (left 1, right 2, middle 4)
    pub fn pointer(&self, buttons: u8, x: u16, y: u16) {
        let _ = self.session.pointer(buttons, x, y);
    }

    /// Trackpad scroll. The delta is in points, positive up and left. The scroll happens at the last pointer position.
    ///
    /// - Send `Ended` when the fingers lift. The session continues with momentum.
    pub fn scroll(&self, phase: Phase, dx: f32, dy: f32) {
        let _ = self.session.scroll(phase, (dx, dy));
    }

    /// Mouse wheel, with the same direction as scroll. The scroll happens at the last pointer position.
    ///
    /// - `dx` and `dy` are the distance in points, converted from the client's own distance with the same scale as scroll.
    /// - `lines_x` and `lines_y` are the same distance in lines.
    /// - Send what this system's wheel would scroll, from its settings or defaults. Mac apps use either the distance or
    ///   the lines, and the Mac adds no acceleration.
    pub fn wheel(&self, dx: f32, dy: f32, lines_x: f32, lines_y: f32) {
        let _ = self.session.wheel((dx, dy), (lines_x, lines_y));
    }

    /// Magnify gesture. The scale starts at 1.0 and is cumulative.
    pub fn magnify(&self, phase: Phase, scale: f32) {
        let _ = self.session.magnify(phase, scale);
    }

    /// One finger on the screen. `id` is distinct per finger, and the position is in layout pixels like the pointer.
    ///
    /// - The whole screen acts as a trackpad. The cursor position arrives as a `Pointer` event.
    pub fn touch(&self, id: u32, phase: TouchPhase, x: f32, y: f32) {
        let _ = self.session.touch(id, phase, (x, y));
    }

    /// Puts text on the Mac's clipboard. Past the same size limit used when receiving, nothing is sent and `false` is returned.
    pub fn clipboard(&self, text: String) -> bool {
        self.session.clipboard(&text).is_ok()
    }

    /// Requests a keyframe. Call it after recreating the decoder or dropping frames because input backed up.
    pub fn request_keyframe(&self) {
        let _ = self.session.request_keyframe();
    }

    /// Resizes the virtual display. A new layout and decoder config follow.
    pub fn resize(&self, mode: DisplayMode) {
        let _ = self.session.resize(&mode.into());
    }

    /// Closes the session. Named apart from the Kotlin object's `close` (native resource release) to avoid a clash.
    pub fn disconnect(&self) {
        self.session.close();
    }
}

/// What to send so the Mac types what this Latin key means in `language` (a BCP 47 tag such as `ko-KR`), given the Mac's
/// input source from the latest `Event::Source`.
///
/// - `latin` is the keysym of the Latin character on the physical key, shifted as typed.
/// - `None` when the language has no Mac layout to follow, such as Latin-script languages: send what the keyboard produced.
/// - In secure input it returns the Latin key, since the Mac forces ABC there.
/// - Not for shortcuts: with Command, Control, or Option held, send the Latin key as is.
#[uniffi::export]
pub fn character(latin: u32, language: String, source: Option<InputSource>) -> Option<u32> {
    crate::keyboard::character(latin, &language, source.as_ref())
}
