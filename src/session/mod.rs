//! A session with one server. It owns the sockets and handles connecting, authentication, media negotiation, reception,
//! and reconnecting.
//!
//! - `Session::start` returns immediately and reports progress and results as events. The last event is always
//!   `Closed`.
//! - The native layer only receives complete access units and decoder configs. Report decoder errors with
//!   `request_keyframe`.
//! - When media does not start, the session ends as Apple's client does. Only HEVC that brought no video at all is
//!   retried once from scratch with H.264.
use crate::error::{Error, Result};
use crate::media::{AccessUnit, AudioFrame, DecoderConfig, VideoCodec};
use crate::message::clipboard::{self, ClipboardItem};
use crate::message::gesture::{MomentumPhase, Phase};
use crate::message::{CursorImage, DisplayConfig, InputSource, Layout, client};
use control::Control;
use engine::Engine;
use media::Sockets;
use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinSet;

mod control;
mod engine;
mod input;
mod media;
mod negotiation;
mod switching;

const RETRY_DELAY: Duration = Duration::from_millis(800);

// The session waits when the event receiver is slow, so video frames do not pile up in memory.
const EVENT_CAPACITY: usize = 256;

/// Connection settings.
pub struct Config {
    /// Host name or address. It is resolved to IPv4, and the same address is used for TCP and UDP.
    pub host: String,
    /// The server's TCP port. Audio arrives on the UDP port with the same number and video on the next one.
    pub port: u16,
    pub username: String,
    pub password: String,
    /// Server public key (DER) saved from a previous connection. A mismatch stops the connection with `KeyChanged`.
    pub known_key: Option<Vec<u8>>,
    /// Virtual display config. With one the Mac's physical screen is hidden; without one the physical screen is
    /// streamed as is.
    pub display: Option<DisplayConfig>,
    /// Whether to receive audio. Turn it off on devices without an AAC-ELD decoder.
    pub audio: bool,
    /// Video codec to request. Request HEVC 4:4:4 only when the device's hardware decoder supports it. If it brings no
    /// video at all, the retry uses H.264.
    pub codec: VideoCodec,
    /// Local UDP ports (audio, video). They default to the server's numbers; set them only when those are unavailable,
    /// such as when testing on the server's own machine.
    pub local_ports: Option<(u16, u16)>,
}

impl Config {
    /// Builds settings for the default port (5900) without audio or a virtual display.
    pub fn new(host: impl Into<String>, username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 5900,
            username: username.into(),
            password: password.into(),
            known_key: None,
            display: None,
            audio: false,
            codec: VideoCodec::H264,
            local_ports: None,
        }
    }
}

/// A connection stage. Showing the user which stage failed makes the cause easier to find.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
#[non_exhaustive]
pub enum Stage {
    /// Warm-up TCP connection
    Warmup,
    /// RFB version and security type exchange
    Handshake,
    /// RSA-SRP authentication
    Auth,
    /// Plaintext prelude and record layer activation
    KeyExchange,
    /// Media offer exchange
    Offer,
    /// Waiting for media
    Media,
}

impl fmt::Display for Stage {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(match self {
            Self::Warmup => "warm-up",
            Self::Handshake => "handshake",
            Self::Auth => "authentication",
            Self::KeyExchange => "key exchange",
            Self::Offer => "media negotiation",
            Self::Media => "media reception",
        })
    }
}

/// An event the session reports.
#[derive(Debug)]
#[non_exhaustive]
pub enum Event {
    /// This stage has started.
    Progress(Stage),
    /// The server's public key (DER). On a first connection, save it and pass it as `known_key` next time.
    ServerKey(Vec<u8>),
    /// Authentication succeeded. `server_proof` tells whether the server's SRP proof (M2) was verified.
    Authenticated { server_proof: bool },
    /// Display layout, the reference for pointer coordinates and decoder output
    Layout(Layout),
    /// New decoder config. Recreate the decoder with it.
    DecoderConfig(DecoderConfig),
    /// Audio decoder config (AudioSpecificConfig). Arrives before the first audio of each audio stream; recreate the
    /// audio decoder with it.
    AudioConfig(Vec<u8>),
    /// Audio access unit
    Audio(AudioFrame),
    /// Access unit for the decoder. Only arrives after a decoder config.
    Frame(AccessUnit),
    /// Every band of one instant has arrived as frames, so show the screen they make up. Only for video in several
    /// bands, and not while any band's latest picture is lost or broken, so the screen never pairs bands from different
    /// moments.
    Present,
    /// Cursor shape, used as the pointer icon. It also shows a hidden cursor again.
    Cursor(Arc<CursorImage>),
    /// The Mac hid its cursor, as it does while typing or over playing video. Hide the pointer until the next
    /// `Cursor`, which also comes once the pointer moves.
    CursorHidden,
    /// The Mac's computer name. Arrives once after authentication.
    ServerName(String),
    /// The Mac's current input source, which decides what keys type there
    InputSource(InputSource),
    /// The Mac's clipboard changed to content of these kinds (UTIs). Fetch the data with `fetch_clipboard` when it is
    /// needed, as when it is pasted.
    ClipboardOffer(Vec<String>),
    /// The Mac's clipboard data, one item per kind, answering `fetch_clipboard`
    Clipboard(Vec<ClipboardItem>),
    /// The Mac wants the data of the clipboard offered with `offer_clipboard`, as when it is pasted there. Answer with
    /// `send_clipboard`.
    ClipboardRequested,
    /// No frame has come out for too long, so the decoder must be recreated. The session requests the keyframe.
    ResetDecoder,
    /// Media reception stats. Arrives every 0.5 seconds after media negotiation.
    Stats(MediaStats),
    /// The session ended. `None` when it was closed with `close()`.
    Closed(Option<Error>),
}

/// Media reception stats. When the screen freezes, they tell whether the server stopped sending or received packets
/// were dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct MediaStats {
    /// Datagrams received on the video port from the server address, including RTCP
    pub datagrams: u64,
    /// RTP packets among them that passed SRTP authentication
    pub authenticated: u64,
    /// Size of the largest of those datagrams. A path with a smaller MTU, such as a VPN, drops larger ones.
    pub largest_bytes: u32,
    /// SSRC currently being decoded
    pub ssrc: Option<u32>,
    /// Highest extended sequence number received on that SSRC
    pub highest: Option<u32>,
    /// Number of other SSRCs being watched as switch candidates
    pub candidates: u32,
    /// Number of offers sent, including those resent after negotiation for resizes and keyframes
    pub offers: u32,
    /// Number of sequence numbers found missing. The server does not retransmit, so these are lost unless they arrive
    /// late.
    pub missing: u64,
    /// Number of packets that filled a gap by arriving out of order
    pub filled: u64,
    /// Number of access units that could not be recovered and were dropped
    pub losses: u64,
    /// How long media is currently held back so that a network that pauses doesn't make it skip
    pub delay_ms: u32,
}

/// The channel the session sends events through. It remembers the last announced stage and reports failures as failures
/// of that stage.
struct Events {
    sender: mpsc::Sender<Event>,
    stage: Stage,
}

impl Events {
    fn new(sender: mpsc::Sender<Event>) -> Self {
        Self {
            sender,
            stage: Stage::Warmup,
        }
    }

    /// Sends an event, failing with `Closed` when the receiver is gone.
    async fn send(&mut self, event: Event) -> Result<()> {
        if let Event::Progress(stage) = event {
            self.stage = stage;
        }

        self.sender.send(event).await.map_err(|_| Error::Closed)
    }

    /// Sends all queued events in order.
    async fn publish(&mut self, outgoing: &mut Vec<Event>) -> Result<()> {
        for event in outgoing.drain(..) {
            self.send(event).await?;
        }

        Ok(())
    }

    /// Wraps a failure as a failure of the last announced stage.
    fn failed(&self, source: Error) -> Error {
        Error::Failed {
            stage: self.stage,
            source: Box::new(source),
        }
    }
}

/// A command the native layer sends to the session.
enum Command {
    Key {
        keysym: u32,
        down: bool,
    },
    /// An input and when it happened. Double clicks are judged from this time, not from when the engine handled it.
    Input(input::Event, Instant),
    Clipboard(Vec<u8>),
    InputSource(String),
    ReleaseInputSource,
    Keyframe,
    Resize(Vec<u8>),
}

/// Sends input and commands to a running session. Clones refer to the same session.
#[derive(Clone)]
pub struct Session {
    commands: mpsc::UnboundedSender<Command>,
    // Closing is signaled separately from commands, which are only read while the engine runs, so it also works while
    // connecting or waiting to retry.
    closed: Arc<Notify>,
}

impl Session {
    /// Starts a session on the current tokio runtime. Progress arrives on the returned receiver.
    pub fn start(config: Config) -> (Self, mpsc::Receiver<Event>) {
        let (commands, receiver) = mpsc::unbounded_channel();
        let (events, events_receiver) = mpsc::channel(EVENT_CAPACITY);
        let closed = Arc::new(Notify::new());

        tokio::spawn(run(config, receiver, events, closed.clone()));
        (Self { commands, closed }, events_receiver)
    }

    /// Presses or releases a key. The key is an X11 keysym of what the Mac should type; Command is Alt (`0xffe9`) and
    /// Option is Meta (`0xffe7`).
    ///
    /// - It goes to the Mac as is. Choosing it is the client's job; the FFI's `character` gives what a Latin key types
    ///   in a language.
    /// - Release with the keysym that was pressed, so no key stays held on the Mac if the language changed in between.
    /// - `keycode` is the macOS virtual key code of the physical key, which `keyboard::mac_keycode` gives for a USB HID
    ///   usage. With it the key goes as Apple's client sends keys, and the agent presses that key, so the Mac's input
    ///   method composes it even when the agent holds the wrong layout after a source switch. Without one, as for a
    ///   shortcut the client makes up, only the keysym goes and the agent looks it up in its layout.
    pub fn key(&self, keysym: u32, keycode: Option<u16>, down: bool) -> Result<()> {
        match keycode {
            Some(keycode) => self.input(input::Event::Key { keysym, keycode, down }),
            None => self.send(Command::Key { keysym, down }),
        }
    }

    /// Sends the pointer position and button state. Coordinates are pixels in `Layout.pixels`.
    pub fn pointer(&self, buttons: u8, x: u16, y: u16) -> Result<()> {
        self.input(input::Event::Pointer { buttons, x, y })
    }

    /// Trackpad scroll while fingers touch. The delta is in points, and the direction follows CGEvent (positive is up
    /// and left).
    ///
    /// - The scroll happens at the last pointer position.
    /// - As from a Mac trackpad: `MayBegin` when fingers rest, `Began` with the first movement, then `Changed`, and
    ///   `Ended` with no delta when they lift. Fingers that lift without moving end with `Cancelled`.
    pub fn scroll(&self, phase: Phase, delta: (f32, f32)) -> Result<()> {
        self.input(input::Event::Scroll { phase, delta })
    }

    /// Momentum after a trackpad scroll, as the client's system decelerates it. The delta is the same as
    /// [`Self::scroll`].
    ///
    /// - `Began` follows the scroll's `Ended` and carries the first movement. `Ended` has no delta and also ends
    ///   momentum that fingers touching down interrupt, before the next scroll.
    pub fn momentum(&self, phase: MomentumPhase, delta: (f32, f32)) -> Result<()> {
        self.input(input::Event::Momentum { phase, delta })
    }

    /// Mouse wheel, with the same direction as [`Self::scroll`]. The scroll happens at the last pointer position.
    ///
    /// - `points` is the distance and `lines` the same distance in lines, both as the client's system would scroll.
    ///   Mac apps use one or the other, so neither is derived here.
    /// - The Mac adds no acceleration.
    pub fn wheel(&self, points: (f32, f32), lines: (f32, f32)) -> Result<()> {
        self.input(input::Event::Wheel { points, lines })
    }

    /// Magnify gesture. The scale starts at 1.0 and is cumulative. The center is the last pointer position.
    pub fn magnify(&self, phase: Phase, scale: f32) -> Result<()> {
        self.input(input::Event::Magnify { phase, scale })
    }

    /// Selects the Mac's input source by its ID, as Apple's "Sync Keyboard Language" does.
    ///
    /// - Call it as soon as the client's language changes, from the system's notice when there is one, and before the
    ///   first key typed in it. The source last asked for is not asked again, so a source the Mac changes to by itself,
    ///   as when another window takes focus, is left alone until the client's language changes, as with Apple's client.
    /// - The Mac switches at once, and keys sent meanwhile wait until it reports the new source, a few milliseconds.
    pub fn select_input_source(&self, id: &str) -> Result<()> {
        self.send(Command::InputSource(id.to_owned()))
    }

    /// Lets the Mac keep its own input source again, as Apple's client does when "Sync Keyboard Language" is turned
    /// off. The agent otherwise applies the source last asked for at every key, undoing a switch made on the Mac.
    /// Nothing is sent when no source was asked for.
    pub fn release_input_source(&self) -> Result<()> {
        self.send(Command::ReleaseInputSource)
    }

    /// Puts content of these kinds (UTIs) on the Mac's clipboard without its data, as Apple's client does. The Mac
    /// sends `ClipboardRequested` when it needs the data.
    pub fn offer_clipboard(&self, kinds: &[String]) -> Result<()> {
        self.send(Command::Clipboard(clipboard::offer(kinds)?))
    }

    /// Sends clipboard data, one item per kind, either answering `ClipboardRequested` or on its own. Past the same size
    /// limit used when receiving, nothing is sent and `TooLong` is returned.
    pub fn send_clipboard(&self, items: &[ClipboardItem]) -> Result<()> {
        self.send(Command::Clipboard(clipboard::data(items)?))
    }

    /// Requests the data of the Mac's clipboard, which arrives as `Clipboard`.
    pub fn fetch_clipboard(&self) -> Result<()> {
        self.send(Command::Clipboard(client::clipboard_request().to_vec()))
    }

    /// Requests a keyframe, and again until one arrives. Call it once after recreating the decoder or dropping frames
    /// on your own; the caller discards frames until the keyframe arrives, and nothing is acknowledged to the server
    /// meanwhile.
    pub fn request_keyframe(&self) -> Result<()> {
        self.send(Command::Keyframe)
    }

    /// Resizes the virtual display. The size actually applied arrives as `Event::Layout`.
    ///
    /// - With no modes or a pixel size past the server's limit, nothing is sent and `Invalid` is returned.
    pub fn resize(&self, display: &DisplayConfig) -> Result<()> {
        self.send(Command::Resize(display.encode()?))
    }

    /// Closes the session. It stops immediately even while connecting, and `Event::Closed(None)` arrives last.
    pub fn close(&self) {
        self.closed.notify_one();
    }

    fn input(&self, event: input::Event) -> Result<()> {
        self.send(Command::Input(event, Instant::now()))
    }

    fn send(&self, command: Command) -> Result<()> {
        self.commands.send(command).map_err(|_| Error::Closed)
    }
}

async fn run(
    config: Config,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::Sender<Event>,
    closed: Arc<Notify>,
) {
    let mut events = Events::new(events);

    // Closing drops the attempt in progress on the spot, along with its sockets.
    let result = tokio::select! {
        result = attempts(&config, &mut commands, &mut events) => result,
        () = closed.notified() => Ok(()),
    };

    let error = result.err().map(|error| events.failed(error));
    let _ = events.send(Event::Closed(error)).await;
}

async fn attempts(config: &Config, commands: &mut mpsc::UnboundedReceiver<Command>, events: &mut Events) -> Result<()> {
    let result = attempt_session(config, config.codec, commands, events).await;

    // Video packets that arrived without a start mean a lossy path, which another attempt would not fix. No video at
    // all under HEVC may be a Mac that does not send HEVC, so that one case retries with H.264.
    match result {
        Err(Error::NoMedia(stats)) if config.codec == VideoCodec::Hevc && stats.authenticated == 0 => {
            tokio::time::sleep(RETRY_DELAY).await;
            attempt_session(config, VideoCodec::H264, commands, events).await
        }
        result => result,
    }
}

async fn attempt_session(
    config: &Config,
    codec: VideoCodec,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    events: &mut Events,
) -> Result<()> {
    events.send(Event::Progress(Stage::Warmup)).await?;
    let server = resolve(&config.host, config.port).await?;
    let local_ports = config.local_ports.unwrap_or((config.port, config.port + 1));
    let sockets = Sockets::bind(server, config.port, local_ports)?;

    // Send UDP during the handshake so the local firewall accepts the server's media as replies. It stops once the
    // record layer is on. It lives in a JoinSet so it also stops when the attempt is dropped midway.
    let mut punch = JoinSet::new();
    punch.spawn(sockets.clone().punch());
    let control = Control::connect(config, server, events).await;
    punch.abort_all();

    Engine::new(control?, sockets, config, codec)
        .run(commands, events)
        .await
}

async fn resolve(host: &str, port: u16) -> Result<IpAddr> {
    // Resolve to IPv4 only, because a `.local` name resolved to IPv6 connects over TCP but UDP media never arrives.
    let mut addresses = tokio::net::lookup_host((host, port)).await?;
    let address = addresses.find(|address| address.is_ipv4());
    address
        .map(|address| address.ip())
        .ok_or_else(|| Error::Resolve(host.into()))
}

#[cfg(test)]
mod tests {
    use super::{Event, Events, Stage};
    use crate::error::Error;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn failure_is_reported_at_the_last_announced_stage() {
        let (sender, mut receiver) = mpsc::channel(4);
        let mut events = Events::new(sender);

        events.send(Event::Progress(Stage::Handshake)).await.unwrap();
        events.send(Event::Progress(Stage::Auth)).await.unwrap();
        events.send(Event::Authenticated { server_proof: true }).await.unwrap();

        let error = events.failed(Error::Rejected);
        assert!(
            matches!(error, Error::Failed { stage: Stage::Auth, ref source } if matches!(**source, Error::Rejected))
        );
        assert!(matches!(receiver.recv().await, Some(Event::Progress(Stage::Handshake))));
    }
}
