//! A session with one server. It owns the sockets and handles connecting, authentication, media negotiation, reception, and reconnecting.
//!
//! - `Session::start` returns immediately and reports progress and results as events. The last event is always `Closed`.
//! - The native layer only receives complete access units and decoder configs. Report decoder errors with `request_keyframe`.
//! - When media does not start, the session disconnects and connects again from scratch, up to three attempts in total.
use crate::error::{Error, Result};
use crate::media::{AccessUnit, AudioFrame, DecoderConfig, Priority, VideoCodec};
use crate::message::gesture::Phase;
use crate::message::{CursorImage, DisplayConfig, InputSource, Layout, clipboard};
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
pub use input::TouchPhase;
mod keyboard;
mod media;
mod negotiation;

const ATTEMPTS: u32 = 3;
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
    /// Virtual display config. With one the Mac's physical screen is hidden; without one the physical screen is streamed as is.
    pub display: Option<DisplayConfig>,
    /// Whether to receive audio. Turn it off on devices without an AAC-ELD decoder.
    pub audio: bool,
    /// Whether latency or quality wins when packets are lost
    pub priority: Priority,
    /// Video codec to request. Request HEVC 4:4:4 only when the device's hardware decoder supports it. If video does not start, the retry uses H.264.
    pub codec: VideoCodec,
    /// Local UDP ports (audio, video). They default to the server's numbers; set them only when those are unavailable, such as when testing on the server's own machine.
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
            priority: Priority::default(),
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
    /// Audio decoder config (AudioSpecificConfig). Arrives once before the first audio.
    AudioConfig(Vec<u8>),
    /// Audio access unit
    Audio(AudioFrame),
    /// Access unit for the decoder. Only arrives after a decoder config, and resumes from a keyframe after a loss.
    Frame(AccessUnit),
    /// Cursor shape, used as the pointer icon. The position arrives as `Pointer` only when finger touches moved it.
    Cursor(Arc<CursorImage>),
    /// Cursor position moved by touch, in layout pixels. Draw the cursor here.
    Pointer { x: u16, y: u16 },
    /// The Mac's computer name. Arrives once after authentication.
    ServerName(String),
    /// The Mac's current input source. The session does character conversion itself, so this is only for logging and display.
    InputSource(InputSource),
    /// The Mac's clipboard text
    Clipboard(String),
    /// No frame has come out for too long, so the decoder must be recreated. The session requests the keyframe.
    ResetDecoder,
    /// Media reception stats. Arrives every 0.5 seconds after media negotiation.
    Stats(MediaStats),
    /// The session ended. `None` when it was closed with `close()`.
    Closed(Option<Error>),
}

/// Media reception stats. When the screen freezes, they tell whether the server stopped sending or received packets were dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct MediaStats {
    /// Datagrams received on the video port from the server address, including RTCP
    pub datagrams: u64,
    /// RTP packets among them that passed SRTP authentication
    pub authenticated: u64,
    /// SSRC currently being decoded
    pub ssrc: Option<u32>,
    /// Highest extended sequence number received on that SSRC
    pub highest: Option<u32>,
    /// Number of other SSRCs being watched as switch candidates
    pub candidates: u32,
    /// Number of offers sent, including those resent after negotiation for resizes and keyframes
    pub offers: u32,
    /// Number of sequence numbers for which retransmission was requested
    pub nacked: u64,
    /// Number of packets that filled a gap. Grows as retransmissions arrive.
    pub filled: u64,
    /// Number of unrecovered losses that forced a wait for a keyframe
    pub losses: u64,
}

/// The channel the session sends events through. It remembers the last announced stage and reports failures as failures of that stage.
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
    /// An input and when it happened. Momentum velocity is computed from this time, not from when the engine handled it.
    Input(input::Event, Instant),
    Language(String),
    Clipboard(Vec<u8>),
    Keyframe,
    Resize(Vec<u8>),
}

/// Sends input and commands to a running session. Clones refer to the same session.
#[derive(Clone)]
pub struct Session {
    commands: mpsc::UnboundedSender<Command>,
    // Closing is signaled separately from commands, which are only read while the engine runs, so it also works while connecting or waiting to retry.
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

    /// Presses or releases a key. The key is an X11 keysym; Command is Super (`0xffeb`) and Option is Alt (`0xffe9`).
    ///
    /// - Send character keys as the Latin letter of the physical key. When the Mac's input source is Korean, the session converts them to Dubeolsik jamo.
    pub fn key(&self, keysym: u32, down: bool) -> Result<()> {
        self.send(Command::Key { keysym, down })
    }

    /// Sends the pointer position and button state. Coordinates are pixels in `Layout.pixels`.
    pub fn pointer(&self, buttons: u8, x: u16, y: u16) -> Result<()> {
        self.input(input::Event::Pointer { buttons, x, y })
    }

    /// Trackpad scroll. The delta is in points, and the direction follows CGEvent (positive is up and left).
    ///
    /// - The scroll happens at the last pointer position.
    /// - After `Ended`, momentum continues at the velocity just before the fingers lifted. The delta with `Ended` is usually zero.
    pub fn scroll(&self, phase: Phase, delta: (f32, f32)) -> Result<()> {
        self.input(input::Event::Scroll { phase, delta })
    }

    /// Mouse wheel, in notches, with the same direction as [`Self::scroll`].
    pub fn wheel(&self, lines: (f32, f32)) -> Result<()> {
        self.input(input::Event::Wheel { lines })
    }

    /// Magnify gesture. The scale starts at 1.0 and is cumulative. The center is the last pointer position.
    pub fn magnify(&self, phase: Phase, scale: f32) -> Result<()> {
        self.input(input::Event::Magnify { phase, scale })
    }

    /// One finger on the screen. The position is in layout pixels like the pointer and is not clamped to the screen.
    ///
    /// - The whole screen acts as a trackpad. One finger moves the cursor, taps to click, and drags after a tap; two fingers right-click, scroll, and magnify.
    /// - The cursor moves by relative motion rather than to the finger, and its new position arrives as [`Event::Pointer`].
    pub fn touch(&self, id: u32, phase: TouchPhase, at: (f32, f32)) -> Result<()> {
        self.input(input::Event::Touch(input::Touch { id, phase, at }))
    }

    /// Puts text on the Mac's clipboard. Past the same size limit used when receiving, nothing is sent and `TooLong` is returned.
    pub fn clipboard(&self, text: &str) -> Result<()> {
        self.send(Command::Clipboard(clipboard::message(text)?))
    }

    /// Requests a keyframe. Call it after recreating the decoder or dropping frames. The caller discards frames until the keyframe arrives.
    pub fn request_keyframe(&self) -> Result<()> {
        self.send(Command::Keyframe)
    }

    /// Resizes the virtual display. The size actually applied arrives as `Event::Layout`.
    ///
    /// - With no modes or a pixel size past the server's limit, nothing is sent and `Invalid` is returned.
    pub fn resize(&self, display: &DisplayConfig) -> Result<()> {
        self.send(Command::Resize(display.encode()?))
    }

    /// Reports the tablet's input language. Call it with a BCP 47 tag (`ko-KR`) whenever the language changes.
    ///
    /// - Characters are converted to the matching Mac layout's characters before sending. The Mac's input source is not changed, so leave the Mac on the native-language input method.
    /// - Meant for platforms whose keys only give the Latin letter of the physical position (Android). Platforms that give layout-applied characters (iPadOS) do not call it and send those characters as is.
    pub fn language(&self, language: &str) -> Result<()> {
        self.send(Command::Language(language.to_owned()))
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
    let mut attempt = 1;
    let mut codec = config.codec;

    loop {
        let result = attempt_session(config, codec, commands, events).await;

        if !matches!(result, Err(Error::NoMedia { .. })) || attempt == ATTEMPTS {
            return result;
        }

        // If HEVC video does not start, later attempts use H.264, because not every Mac has been confirmed to accept single-tile HEVC.
        if codec == VideoCodec::Hevc {
            codec = VideoCodec::H264;
        }

        attempt += 1;
        tokio::time::sleep(RETRY_DELAY).await;
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

    // Send UDP during the handshake so the local firewall accepts the server's media as replies. It stops once the record layer is on.
    // It lives in a JoinSet so it also stops when the attempt is dropped midway.
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
