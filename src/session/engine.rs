use super::control::Control;
use super::input::Input;
use super::media::{Media, Sockets};
use super::negotiation::Negotiation;
use super::switching::Switching;
use super::{Command, Config, Event, Events, Stage};
use crate::error::{Error, Result};
use crate::media::VideoCodec;
use crate::message::{Cursors, InputSource, Layout, Messages, Rect, ServerMessage, client, clipboard};
use crate::offer::Offer;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

const TICK: Duration = Duration::from_millis(500);
// The receiver feedback interval, as in Apple's client. The server raises its bitrate one step per feedback.
const FEEDBACK: Duration = Duration::from_millis(50);

// If no decoder config arrives this long after the answer, the session gives up, about as long as Apple's client waits.
// On a lossy path the first keyframe often arrives incomplete, and the keyframes asked for meanwhile are more chances.
const MEDIA_START: Duration = Duration::from_secs(10);

/// Session state once the record layer is on. One task handles the control channel, UDP, timers, and commands.
///
/// - Record sequence numbers and CBC state are tied to send order, so only this task sends on the control channel.
pub struct Engine {
    control: Control,
    sockets: Sockets,
    media: Media,
    input: Input,
    negotiation: Negotiation,
    messages: Messages,
    cursors: Cursors,
    // Pixel size of the last layout. A change resends the offer.
    pixels: Option<(u16, u16)>,
    // The latest resize asked before the first frame, sent once it arrives, since its framebuffer request derails the start
    display: Option<Vec<u8>>,
    // An input source the Mac is switching to, holding keys until it reports it
    switching: Option<Switching>,
}

// Whichever awaited source finished first
enum Wake {
    Tcp(Result<Vec<Vec<u8>>>),
    Video(std::io::Result<(usize, SocketAddr)>),
    Audio(std::io::Result<(usize, SocketAddr)>),
    Tick,
    Feedback,
    Retry,
    Deadline,
    Switched,
    Command(Option<Command>),
}

impl Engine {
    /// `codec` is the codec to request in this attempt. If HEVC does not start, the session retries with H.264.
    pub fn new(control: Control, sockets: Sockets, config: &Config, codec: VideoCodec) -> Self {
        let offer = Offer::new(config.audio, codec);

        Self {
            media: Media::new(sockets.clone(), &offer, codec, config.priority),
            input: Input::new(control.keys().clone()),
            control,
            sockets,
            negotiation: Negotiation::new(offer),
            messages: Messages::default(),
            cursors: Cursors::default(),
            pixels: None,
            display: None,
            switching: None,
        }
    }

    pub async fn run(mut self, commands: &mut mpsc::UnboundedReceiver<Command>, events: &mut Events) -> Result<()> {
        events.send(Event::Progress(Stage::Offer)).await?;

        let mut outgoing = Vec::new();
        self.start(&mut outgoing).await?;
        events.publish(&mut outgoing).await?;

        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut feedback = tokio::time::interval(FEEDBACK);
        feedback.set_missed_tick_behavior(MissedTickBehavior::Delay);

        let mut tcp_buffer = vec![0; 64 << 10];
        let mut video_buffer = vec![0; 2048];
        let mut audio_buffer = vec![0; 2048];
        let (video, audio) = (self.sockets.video.clone(), self.sockets.audio.clone());

        loop {
            let deadline = self.media.deadline();
            let retry = self.negotiation.deadline();
            let switched = self.switching.as_ref().map(Switching::deadline);

            let wake = tokio::select! {
                opened = self.control.receive(&mut tcp_buffer) => Wake::Tcp(opened),
                received = video.recv_from(&mut video_buffer) => Wake::Video(received),
                received = audio.recv_from(&mut audio_buffer) => Wake::Audio(received),
                _ = ticker.tick() => Wake::Tick,
                _ = feedback.tick() => Wake::Feedback,
                _ = sleep_until(retry) => Wake::Retry,
                _ = sleep_until(deadline) => Wake::Deadline,
                _ = sleep_until(switched) => Wake::Switched,
                command = commands.recv() => Wake::Command(command),
            };

            let now = Instant::now();

            match wake {
                Wake::Tcp(opened) => self.receive(opened?, now, &mut outgoing).await?,
                Wake::Video(received) => {
                    let (len, from) = received?;
                    self.media.receive(&video_buffer[..len], from, now, &mut outgoing).await;
                }
                Wake::Audio(received) => {
                    let (len, from) = received?;
                    self.media.receive_audio(&audio_buffer[..len], from, &mut outgoing);
                }
                Wake::Tick => self.tick(now, &mut outgoing).await?,
                Wake::Feedback => self.media.feedback(now).await,
                Wake::Retry => self.retry(now).await?,
                Wake::Deadline => self.media.drain(now, &mut outgoing).await,
                Wake::Switched => self.switched().await?,
                Wake::Command(None) => return Ok(()),
                Wake::Command(Some(command)) => self.command(command, now).await?,
            }

            events.publish(&mut outgoing).await?;
        }
    }

    // Requests sent right after the record layer turns on. After the offer, a non-incremental full-area request wakes the TCP side.
    async fn start(&mut self, outgoing: &mut Vec<Event>) -> Result<()> {
        // The encoding list is sent again over the record layer even though the plaintext prelude had it, matching Apple's client order.
        self.control.send(&client::set_encodings()).await?;
        self.control.send(&client::AUTO_CLIPBOARD).await?;
        self.control.send(&self.negotiation.start(Instant::now())).await?;
        self.control
            .send(&client::framebuffer_request(false, u16::MAX, u16::MAX))
            .await?;

        let (early, opened) = self.control.backlog()?;
        self.rects(early, Instant::now(), outgoing).await?;
        self.receive(opened, Instant::now(), outgoing).await
    }

    async fn receive(&mut self, bodies: Vec<Vec<u8>>, now: Instant, outgoing: &mut Vec<Event>) -> Result<()> {
        for body in bodies {
            self.messages.push(&body);
            self.dispatch(now, outgoing).await?;
        }

        Ok(())
    }

    async fn dispatch(&mut self, now: Instant, outgoing: &mut Vec<Event>) -> Result<()> {
        while let Some(message) = self.messages.pop()? {
            match message {
                ServerMessage::FramebufferUpdate(rects) => self.rects(rects, now, outgoing).await?,
                ServerMessage::ClipboardChanged => self.control.send(&client::clipboard_request()).await?,
                // A malformed clipboard body does not misalign message boundaries, so only that content is dropped and the session stays up.
                ServerMessage::Clipboard(message) => {
                    if let Ok(Some(text)) = clipboard::text(&message) {
                        outgoing.push(Event::Clipboard(text));
                    }
                }
                ServerMessage::Status(_) | ServerMessage::Ignored(_) => {}
            }
        }

        Ok(())
    }

    // A malformed rectangle body (cursor image, layout) drops only that rectangle; message boundaries were already checked.
    async fn rects(&mut self, rects: Vec<Rect>, now: Instant, outgoing: &mut Vec<Event>) -> Result<()> {
        for rect in rects {
            match rect {
                Rect::KeyExchange(_) => return Err(Error::Rekey),
                Rect::Cursor {
                    hotspot,
                    size,
                    cache_id,
                    data,
                } => {
                    if let Ok(Some(image)) = self.cursors.apply(hotspot, size, cache_id, &data) {
                        outgoing.push(Event::Cursor(image));
                    }

                    // Requesting right after a cursor rectangle keeps the next shape updates coming.
                    self.control.send(&client::framebuffer_request(true, 1, 1)).await?;
                }
                Rect::Layout(body) => {
                    let Ok(layout) = Layout::parse(&body) else {
                        continue;
                    };

                    let resized = self
                        .pixels
                        .replace(layout.pixels)
                        .is_some_and(|previous| previous != layout.pixels);
                    self.input.resize(layout.pixels);
                    outgoing.push(Event::Layout(layout));

                    // Request even for a same-size layout (login, lock, session switch), or the cursor shape freezes.
                    self.control
                        .send(&client::framebuffer_request(false, u16::MAX, u16::MAX))
                        .await?;

                    if let Some(offer) = resized.then(|| self.negotiation.resize(now)).flatten() {
                        self.control.send(&offer).await?;
                    }
                }
                // A media answer with a size moves on to the media stage.
                Rect::MediaStream(body) | Rect::Unknown { rest: body, .. } => {
                    if self.negotiation.answer(&body, now) {
                        outgoing.push(Event::Progress(Stage::Media));
                    }
                }
                Rect::InputSource(body) => {
                    let Some(source) = InputSource::parse(&body) else {
                        continue;
                    };

                    if self
                        .switching
                        .as_ref()
                        .is_some_and(|switching| switching.done(&source.id))
                    {
                        self.switched().await?;
                    }

                    outgoing.push(Event::InputSource(source));
                }
                Rect::Skipped(_) => {}
            }
        }

        Ok(())
    }

    async fn tick(&mut self, now: Instant, outgoing: &mut Vec<Event>) -> Result<()> {
        self.media.tick().await;

        let Some(answered) = self.negotiation.answered() else {
            return Ok(());
        };

        outgoing.push(Event::Stats(self.media.stats(self.negotiation.offers())));

        if !self.media.started() {
            if now.duration_since(answered) > MEDIA_START {
                return Err(self.abort());
            }

            return Ok(());
        }

        // No framebuffer requests until the first keyframe, since an update without parameter sets derails the start.
        if self.media.last_frame().is_some() {
            self.control.send(&client::framebuffer_request(true, 1, 1)).await?;

            if let Some(display) = self.display.take() {
                self.resize(&display).await?;
            }
        }

        self.media.watch(now, answered, outgoing).await;
        Ok(())
    }

    async fn command(&mut self, command: Command, now: Instant) -> Result<()> {
        match command {
            Command::Key { keysym, down } => match &mut self.switching {
                Some(switching) => {
                    switching.hold(client::key(keysym, down));
                    Ok(())
                }
                None => self.control.send(&client::key(keysym, down)).await,
            },
            Command::InputSource(id) => {
                let (switching, messages) = Switching::start(&id, self.switching.take(), now);
                self.switching = Some(switching);

                for message in messages {
                    self.control.send(&message).await?;
                }

                Ok(())
            }
            Command::Input(event, at) => match self.input.handle(event, at) {
                Some(message) => self.control.send(&message).await,
                None => Ok(()),
            },
            Command::Clipboard(message) => self.control.send(&message).await,
            // A server gone quiet on a still screen ignores keyframe requests.
            // Resend the offer instead; the server opens a new stream and sends a config and keyframe even for a still screen.
            Command::Keyframe if !self.media.active(now) => match self.negotiation.reoffer(now) {
                Some(offer) => self.control.send(&offer).await,
                None => Ok(()),
            },
            Command::Keyframe => {
                self.media.request_keyframe(now).await;
                Ok(())
            }
            Command::Resize(display) if self.media.last_frame().is_none() => {
                self.display = Some(display);
                Ok(())
            }
            Command::Resize(display) => {
                // A newer size replaces one still held for the first frame.
                self.display = None;
                self.resize(&display).await
            }
        }
    }

    // Sends the keys held for an input source switch, once the Mac reported it or stopped being waited for.
    async fn switched(&mut self) -> Result<()> {
        let Some(switching) = self.switching.take() else {
            return Ok(());
        };

        for key in switching.release() {
            self.control.send(&key).await?;
        }

        Ok(())
    }

    async fn resize(&mut self, display: &[u8]) -> Result<()> {
        self.control.send(display).await?;
        self.control
            .send(&client::framebuffer_request(false, u16::MAX, u16::MAX))
            .await
    }

    // Resend the same offer until an answer with a size arrives, giving up past the limit. After negotiation, send the deferred resend.
    async fn retry(&mut self, now: Instant) -> Result<()> {
        match self.negotiation.expire(now) {
            Some(offer) => self.control.send(&offer).await,
            None => Err(self.abort()),
        }
    }

    // When media does not start, reset the connection immediately (RST). Reconnecting only works once no session lingers on the server.
    fn abort(&self) -> Error {
        self.control.abort();
        Error::NoMedia(self.media.stats(self.negotiation.offers()))
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending().await,
    }
}
