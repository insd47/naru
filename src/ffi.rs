//! The UniFFI boundary that exposes sessions to Kotlin, Swift, and C#.
//!
//! - The native side opens a session by passing a [`SessionListener`] to [`Session::connect`] and receives events in
//!   order on a dedicated session thread.
//! - Core types without tuples or private fields pass through as is; the rest are reshaped here for the native side.
use crate::touchpad::TouchPhase;
use crate::{
    AccessUnit, AudioFrame, ClipboardItem, DecoderConfig, Error, InputSource, MediaStats, Mode, MomentumPhase, Phase,
    Stage, VideoCodec,
};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;
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
    /// Host name or address. It is resolved to IPv4, and the same address is used for TCP and UDP.
    pub host: String,
    /// The Mac's TCP port. Audio arrives on the UDP port with the same number and video on the next one.
    pub port: u16,
    /// Account on the Mac
    pub username: String,
    pub password: String,
    /// Server public key (DER) saved from a previous connection. A mismatch stops the connection.
    pub known_key: Option<Vec<u8>>,
    /// Virtual display. Without one, the Mac's physical screen is streamed.
    pub display: Option<DisplayMode>,
    /// Whether to receive audio. Turn it off on devices without an AAC-ELD decoder.
    pub audio: bool,
    /// Video codec to request. Ask for HEVC 4:4:4 only when the device's hardware decoder supports 4:4:4; if it brings
    /// no video at all, the session retries once with H.264 4:2:0.
    pub codec: VideoCodec,
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
            codec: config.codec,
            local_ports: None,
        }
    }
}

/// Logical size and scale of the virtual display. The pixel size is logical size × scale.
///
/// - A pixel size past the server's limit (3840×2160 landscape, 2160×3840 portrait) is shrunk with the aspect ratio
///   kept. The actual size arrives as `Layout`.
#[derive(uniffi::Record)]
pub struct DisplayMode {
    /// Logical size in points
    pub width: u32,
    pub height: u32,
    /// Pixels per point. 2 is Retina.
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
    /// This stage has started.
    Progress { stage: Stage },
    /// Server public key (DER). On a first connection, save it and pass it as `known_key` next time.
    ServerKey { der: Vec<u8> },
    /// Authentication succeeded. `server_proof` tells whether the server's SRP proof (M2) was verified.
    Authenticated { server_proof: bool },
    /// The Mac's computer name. Arrives once after authentication.
    ServerName { name: String },
    /// Display layout. Pointer coordinates are sent relative to the pixel size.
    Layout {
        logical_width: u16,
        logical_height: u16,
        pixel_width: u16,
        pixel_height: u16,
    },
    // A variant named after the type it carries makes the generated Kotlin nested class shadow that type, so these two
    // are named differently.
    /// New decoder config. Recreate the decoder with it.
    Decoder { config: DecoderConfig },
    /// Audio decoder config (AudioSpecificConfig). Arrives before the first audio of each audio stream; recreate the
    /// audio decoder with it.
    AudioConfig { config: Vec<u8> },
    /// Audio access unit
    Audio { frame: AudioFrame },
    /// Access unit for the decoder
    Frame { unit: AccessUnit },
    /// Every band of one instant has arrived as frames, so show the screen they make up. Only for video in several
    /// bands, and not while any band's latest picture is lost or broken, so the screen never pairs bands from different
    /// moments.
    Present,
    /// Cursor shape as row-major RGBA with non-premultiplied alpha. It also shows a hidden cursor again.
    Cursor {
        width: u16,
        height: u16,
        hotspot_x: u16,
        hotspot_y: u16,
        rgba: Vec<u8>,
    },
    /// The Mac hid its cursor, as it does while typing or over playing video. Hide the pointer until the next
    /// `Cursor`, which also comes once the pointer moves.
    CursorHidden,
    /// The Mac's current input source. Pass the latest one to [`character`].
    Source { source: InputSource },
    /// The Mac's clipboard changed to content of these kinds (UTIs). Fetch the data with `fetch_clipboard` when it is
    /// needed, as when it is pasted.
    ClipboardOffer { kinds: Vec<String> },
    /// The Mac's clipboard data, one item per kind, answering `fetch_clipboard`
    Clipboard { items: Vec<ClipboardItem> },
    /// The Mac wants the data of the clipboard offered with `offer_clipboard`, as when it is pasted there. Answer with
    /// `send_clipboard`.
    ClipboardRequested,
    /// No frame has come out for too long. Recreate the decoder; the session requests the keyframe.
    ResetDecoder,
    /// Media reception stats. Arrives every 0.5 seconds after media negotiation.
    Stats { stats: MediaStats },
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
            crate::Event::Cursor(image) => Self::Cursor {
                width: image.width,
                height: image.height,
                hotspot_x: image.hotspot.0,
                hotspot_y: image.hotspot.1,
                rgba: image.rgba.clone(),
            },
            crate::Event::InputSource(source) => Self::Source { source },
            crate::Event::ClipboardOffer(kinds) => Self::ClipboardOffer { kinds },
            crate::Event::Clipboard(items) => Self::Clipboard { items },
            crate::Event::ClipboardRequested => Self::ClipboardRequested,
            crate::Event::CursorHidden => Self::CursorHidden,
            crate::Event::ResetDecoder => Self::ResetDecoder,
            crate::Event::Present => Self::Present,
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
    /// No video packet arrived at all. Something on the path, such as a firewall, router, or VPN, blocks the Mac's UDP.
    MediaBlocked,
    /// Video arrived, but too much of it was lost to start. The path is too slow or unreliable for a High Performance
    /// connection, as the Mac needs about 21 Mbps at the least.
    SlowNetwork,
    /// The server never started sending video for another reason.
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
            Error::NoMedia(stats) if stats.datagrams == 0 => Self::MediaBlocked,
            // Four bands start through 10% random loss, so more than one packet in ten missing points at the path.
            Error::NoMedia(stats) if stats.missing * 10 > stats.authenticated + stats.missing => Self::SlowNetwork,
            Error::NoMedia(_) => Self::NoMedia,
            _ => Self::Other,
        }
    }
}

/// Receives a session's events.
#[uniffi::export(with_foreign)]
pub trait SessionListener: Send + Sync {
    /// One event. Called in order on the dedicated session thread; the last event is always `Closed`.
    ///
    /// - The next event waits until this returns, and the session waits when events back up. Hand long work to another
    ///   thread.
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
                // UniFFI turns a native exception into a panic. Drop only that event and keep delivering so the final
                // `Closed` arrives.
                while let Some(event) = events.blocking_recv() {
                    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| listener.on_event(event.into())));
                }
            })
            .expect("cannot spawn the event thread");

        Arc::new(Self { session })
    }

    /// Key input. `keysym` is an X11 keysym of what the Mac should type, sent as is; Command is Alt (`0xffe9`) and
    /// Option is Meta (`0xffe7`), and [`character`] gives what a Latin key types in a language. Release with the keysym
    /// that was pressed.
    ///
    /// - `usage` is the physical key as a USB HID keyboard usage (page 7). Like Apple's client, the key goes with its
    ///   macOS key code while the Mac is on the source of the client's language, so the Mac's input method composes it
    ///   even when the agent holds the wrong layout after a source switch. Pass `None` for a key with no physical key,
    ///   such as one of a shortcut the app makes up.
    pub fn key(&self, keysym: u32, usage: Option<u16>, down: bool) {
        let _ = self.session.key(keysym, usage, down);
    }

    /// Pointer position in layout pixels and pressed buttons (left 1, right 2, middle 4)
    pub fn pointer(&self, buttons: u8, x: u16, y: u16) {
        let _ = self.session.pointer(buttons, x, y);
    }

    /// Trackpad scroll while fingers touch. The delta is in points, positive up and left. The scroll happens at the
    /// last pointer position.
    ///
    /// - As from a Mac trackpad: `MayBegin` when fingers rest, `Began` with the first movement, then `Changed`, and
    ///   `Ended` with no delta when they lift. Fingers that lift without moving end with `Cancelled`.
    pub fn scroll(&self, phase: Phase, dx: f32, dy: f32) {
        let _ = self.session.scroll(phase, (dx, dy));
    }

    /// Momentum after a trackpad scroll, with the same delta as scroll. The Mac leaves momentum to the client, so send
    /// what this system's own deceleration would scroll.
    ///
    /// - `Began` follows the scroll's `Ended` and carries the first movement.
    /// - `Ended` has no delta. Send it also when fingers touch down during momentum, before anything else.
    pub fn momentum(&self, phase: MomentumPhase, dx: f32, dy: f32) {
        let _ = self.session.momentum(phase, (dx, dy));
    }

    /// Mouse wheel, with the same direction as scroll. The scroll happens at the last pointer position.
    ///
    /// - `dx` and `dy` are the distance in points, converted from the client's own distance with the same scale as
    ///   scroll.
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

    /// The client's input language as a BCP 47 tag (`ko-KR`). Report it at the start and whenever it changes, from the
    /// system's notice when there is one, and before the first key typed in it.
    ///
    /// - With "Sync Keyboard Language" on, the Mac switches to the source of the language at once, and keys sent
    ///   meanwhile wait until it reports the new source, a few milliseconds. A source the Mac changes to by itself, as
    ///   when another window takes focus, is left alone until the language changes, as with Apple's client.
    /// - It also decides whether keys go with their physical key, as [`Session::key`] describes.
    pub fn set_input_language(&self, language: String) {
        let _ = self.session.set_input_language(&language);
    }

    /// Turns Apple's "Sync Keyboard Language" on or off: whether the Mac's input source follows the client's input
    /// language. Off lets the Mac keep its own source again, as Apple's client does.
    pub fn set_sync_keyboard_language(&self, on: bool) {
        let _ = self.session.set_sync_keyboard_language(on);
    }

    /// Puts content of these kinds (UTIs) on the Mac's clipboard without its data, as Apple's client does. The Mac
    /// sends `ClipboardRequested` when it needs the data.
    pub fn offer_clipboard(&self, kinds: Vec<String>) {
        let _ = self.session.offer_clipboard(&kinds);
    }

    /// Sends clipboard data, one item per kind, either answering `ClipboardRequested` or on its own. Past the same size
    /// limit used when receiving, nothing is sent and `false` is returned.
    pub fn send_clipboard(&self, items: Vec<ClipboardItem>) -> bool {
        self.session.send_clipboard(&items).is_ok()
    }

    /// Requests the data of the Mac's clipboard, which arrives as `Clipboard`.
    pub fn fetch_clipboard(&self) {
        let _ = self.session.fetch_clipboard();
    }

    /// Requests a keyframe, and again until one arrives. Call it once after recreating the decoder or dropping frames
    /// on your own, but not for a new decoder config, which comes with its keyframe, or `ResetDecoder`, which asks for
    /// one.
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

/// Turns fingers on a screen into Mac trackpad gestures. The whole screen is one trackpad.
///
/// - One finger moves the cursor and taps to left-click. Touching down right after a tap and moving drags.
/// - Two fingers tap to right-click and move to scroll or magnify. Three or more fingers are system gestures and are
///   ignored.
/// - It knows nothing of the session. The client moves its own cursor, clamped to the screen, and sends the gestures as
///   pointer, scroll, and magnify. Momentum after a scroll comes from the client's system, as for a trackpad.
#[derive(uniffi::Object)]
pub struct Touchpad {
    touchpad: Mutex<crate::touchpad::Touchpad>,
}

#[uniffi::export]
impl Touchpad {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            touchpad: Mutex::new(crate::touchpad::Touchpad::new()),
        })
    }

    /// One finger. `id` is distinct per finger, and the position is in layout pixels, not clamped to the screen.
    pub fn touch(&self, id: u32, phase: TouchPhase, x: f32, y: f32) -> Vec<Gesture> {
        let touch = crate::touchpad::Touch { id, phase, at: (x, y) };
        let mut touchpad = self.touchpad.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        touchpad
            .handle(touch, Instant::now())
            .into_iter()
            .map(Into::into)
            .collect()
    }
}

/// A trackpad gesture from [`Touchpad`]. Distances are in layout pixels.
#[derive(uniffi::Enum)]
pub enum Gesture {
    /// Moves the cursor by this much.
    Move { dx: f32, dy: f32 },
    /// The pointer's new buttons, with the same bits as `pointer`.
    Buttons { buttons: u8 },
    /// Scroll by the fingers' movement, positive down and right. Divide by layout pixels per point and pass to `scroll`
    /// as is, so the content follows the fingers.
    Scroll { phase: Phase, dx: f32, dy: f32 },
    /// Magnify. The scale starts at 1.0 and is cumulative.
    Magnify { phase: Phase, scale: f32 },
}

impl From<crate::touchpad::Gesture> for Gesture {
    fn from(gesture: crate::touchpad::Gesture) -> Self {
        match gesture {
            crate::touchpad::Gesture::Move((dx, dy)) => Self::Move { dx, dy },
            crate::touchpad::Gesture::Buttons(buttons) => Self::Buttons { buttons },
            crate::touchpad::Gesture::Scroll { phase, delta: (dx, dy) } => Self::Scroll { phase, dx, dy },
            crate::touchpad::Gesture::Magnify { phase, scale } => Self::Magnify { phase, scale },
        }
    }
}

/// What to send so the Mac types what this Latin key means in `language` (a BCP 47 tag such as `ko-KR`), given the
/// Mac's input source from the latest `Event::Source`.
///
/// - `latin` is the keysym of the Latin character on the physical key, shifted as typed.
/// - `None` when the language has no Mac layout to follow, such as Latin-script languages: send what the keyboard
///   produced.
/// - In secure input it returns the Latin key, since the Mac forces ABC there.
/// - Not for shortcuts: with Command, Control, or Option held, send the Latin key as is.
#[uniffi::export]
pub fn character(latin: u32, language: String, source: Option<InputSource>) -> Option<u32> {
    crate::keyboard::character(latin, &language, source.as_ref())
}

#[cfg(test)]
mod tests {
    use super::Failure;
    use crate::{Error, MediaStats};

    fn failure(datagrams: u64, authenticated: u64, missing: u64) -> Failure {
        let stats = MediaStats {
            datagrams,
            authenticated,
            largest_bytes: 1384,
            ssrc: None,
            highest: None,
            candidates: 0,
            offers: 1,
            missing,
            filled: 0,
            losses: 0,
            delay_ms: 0,
        };

        Failure::from(&Error::NoMedia(stats))
    }

    #[test]
    fn media_failures_tell_a_blocked_path_from_a_slow_one() {
        assert!(matches!(failure(0, 0, 0), Failure::MediaBlocked));
        assert!(matches!(failure(490, 487, 291), Failure::SlowNetwork));
        assert!(matches!(failure(1000, 900, 101), Failure::SlowNetwork));
        assert!(matches!(failure(1000, 900, 100), Failure::NoMedia));
        assert!(matches!(failure(40, 0, 0), Failure::NoMedia));
    }
}
