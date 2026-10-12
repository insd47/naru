use super::control::Control;
use super::input::Input;
use super::keyboard::Keyboard;
use super::media::{Media, Sockets};
use super::negotiation::Negotiation;
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

// 응답을 받은 뒤 이 시간 안에 디코더 설정이 오지 않으면 서버의 인코더가 멈춘 것으로 보고 다시 접속합니다.
const MEDIA_START: Duration = Duration::from_secs(3);

/// 레코드 계층이 켜진 뒤의 세션 상태입니다. 한 task에서 제어 채널, UDP, 타이머, 명령을 모두 처리합니다.
///
/// - 레코드의 순번과 CBC 상태는 보낸 순서에 묶이므로, 제어 채널로 보내는 일은 이 task에서만 합니다.
pub struct Engine {
    control: Control,
    sockets: Sockets,
    media: Media,
    input: Input,
    keyboard: Keyboard,
    negotiation: Negotiation,
    messages: Messages,
    cursors: Cursors,
    // 마지막 레이아웃의 픽셀 크기. 바뀌면 오퍼를 다시 보냅니다.
    pixels: Option<(u16, u16)>,
}

// 대기하던 일 가운데 먼저 끝난 것
enum Wake {
    Tcp(Result<Vec<Vec<u8>>>),
    Video(std::io::Result<(usize, SocketAddr)>),
    Audio(std::io::Result<(usize, SocketAddr)>),
    Tick,
    Retry,
    Deadline,
    Momentum,
    Command(Option<Command>),
}

impl Engine {
    /// `codec`은 이번 시도에서 요청할 코덱입니다. HEVC가 시작되지 않으면 세션이 H.264로 바꿔 다시 시도합니다.
    pub fn new(control: Control, sockets: Sockets, config: &Config, codec: VideoCodec) -> Self {
        let offer = Offer::new(config.audio, codec);

        Self {
            media: Media::new(sockets.clone(), &offer, codec, config.priority),
            input: Input::new(control.keys().clone()),
            keyboard: Keyboard::new(),
            control,
            sockets,
            negotiation: Negotiation::new(offer),
            messages: Messages::default(),
            cursors: Cursors::default(),
            pixels: None,
        }
    }

    pub async fn run(mut self, commands: &mut mpsc::UnboundedReceiver<Command>, events: &mut Events) -> Result<()> {
        events.send(Event::Progress(Stage::Offer)).await?;

        let mut outgoing = Vec::new();
        self.start(&mut outgoing).await?;
        events.publish(&mut outgoing).await?;

        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        let mut tcp_buffer = vec![0; 64 << 10];
        let mut video_buffer = vec![0; 2048];
        let mut audio_buffer = vec![0; 2048];
        let (video, audio) = (self.sockets.video.clone(), self.sockets.audio.clone());

        loop {
            let deadline = self.media.deadline();
            let momentum = self.input.deadline();
            let retry = self.negotiation.deadline();

            let wake = tokio::select! {
                opened = self.control.receive(&mut tcp_buffer) => Wake::Tcp(opened),
                received = video.recv_from(&mut video_buffer) => Wake::Video(received),
                received = audio.recv_from(&mut audio_buffer) => Wake::Audio(received),
                _ = ticker.tick() => Wake::Tick,
                _ = sleep_until(retry) => Wake::Retry,
                _ = sleep_until(deadline) => Wake::Deadline,
                _ = sleep_until(momentum) => Wake::Momentum,
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
                Wake::Retry => self.retry(now).await?,
                Wake::Deadline => self.media.drain(now, &mut outgoing).await,
                Wake::Momentum => {
                    if let Some(message) = self.input.tick(now) {
                        self.control.send(&message).await?;
                    }
                }
                Wake::Command(None) => return Ok(()),
                Wake::Command(Some(command)) => self.command(command, now).await?,
            }

            events.publish(&mut outgoing).await?;
        }
    }

    // 레코드 계층이 켜진 직후의 요청입니다. 오퍼를 보낸 뒤 전체 영역을 비증분으로 요청해 TCP 쪽 송신을 깨웁니다.
    async fn start(&mut self, outgoing: &mut Vec<Event>) -> Result<()> {
        // 인코딩 목록은 평문 프렐류드에서 보냈어도 레코드 계층에서 한 번 더 보냅니다. 원 클라이언트의 순서입니다.
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
                // 클립보드 내용의 형식 오류는 경계가 어긋난 것이 아니므로 세션을 끊지 않고 그 내용만 버립니다.
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

    // 사각형 본문의 형식 오류(커서 이미지, 레이아웃)는 그 사각형만 버립니다. 메시지 경계는 이미 확인했습니다.
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

                    // 커서 사각형을 받은 직후에 요청해야 다음 모양 갱신이 이어집니다.
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

                    // 크기가 같은 레이아웃(로그인, 잠금, 세션 전환)에서도 요청하지 않으면 커서 모양이 굳습니다.
                    self.control
                        .send(&client::framebuffer_request(false, u16::MAX, u16::MAX))
                        .await?;

                    if let Some(offer) = resized.then(|| self.negotiation.resize(now)).flatten() {
                        self.control.send(&offer).await?;
                    }
                }
                // 크기가 있는 미디어 응답이 오면 미디어 수신 단계로 넘어갑니다.
                Rect::MediaStream(body) | Rect::Unknown { rest: body, .. } => {
                    if self.negotiation.answer(&body, now) {
                        outgoing.push(Event::Progress(Stage::Media));
                    }
                }
                Rect::InputSource(body) => {
                    let Some(source) = InputSource::parse(&body) else {
                        continue;
                    };

                    self.keyboard.observe(&source);
                    outgoing.push(Event::InputSource(source));
                }
                Rect::Skipped(_) => {}
            }
        }

        Ok(())
    }

    async fn tick(&mut self, now: Instant, outgoing: &mut Vec<Event>) -> Result<()> {
        self.media.tick(now).await;

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

        // 첫 키프레임이 오는 동안에는 프레임버퍼를 요청하지 않습니다. 파라미터 세트가 빠진 갱신으로 시작이 꼬입니다.
        if self.media.last_frame().is_some() {
            self.control.send(&client::framebuffer_request(true, 1, 1)).await?;
        }

        self.media.watch(now, answered, outgoing).await;
        Ok(())
    }

    async fn command(&mut self, command: Command, now: Instant) -> Result<()> {
        match command {
            Command::Key { keysym, down } => self.control.send(&self.keyboard.key(keysym, down)).await,
            Command::Language(language) => {
                self.keyboard.select(&language);
                Ok(())
            }
            Command::Input(event, at) => self.control.send_all(self.input.handle(event, at)).await,
            Command::Clipboard(message) => self.control.send(&message).await,
            // 화면이 멈춰 서버가 조용하면 키프레임 요청에 답하지 않습니다(2026-10-12 실측, 26초 동안 응답 없음).
            // 이때는 오퍼를 다시 보냅니다. 서버는 새 스트림을 열며 화면이 멈춰 있어도 설정과 키프레임을 보냅니다.
            Command::Keyframe if !self.media.active(now) => match self.negotiation.reoffer(now) {
                Some(offer) => self.control.send(&offer).await,
                None => Ok(()),
            },
            Command::Keyframe => {
                self.media.request_keyframe(now).await;
                Ok(())
            }
            Command::Resize(display) => {
                self.control.send(&display).await?;
                self.control
                    .send(&client::framebuffer_request(false, u16::MAX, u16::MAX))
                    .await
            }
        }
    }

    // 크기가 있는 응답을 받을 때까지 같은 오퍼를 다시 보내고, 횟수를 넘으면 포기합니다. 협상 뒤에는 미뤄 둔 재전송을 보냅니다.
    async fn retry(&mut self, now: Instant) -> Result<()> {
        match self.negotiation.expire(now) {
            Some(offer) => self.control.send(&offer).await,
            None => Err(self.abort()),
        }
    }

    // 미디어가 시작되지 않으면 연결을 즉시 끊습니다(RST). 서버 쪽 세션이 남지 않게 해야 다시 접속할 수 있습니다.
    fn abort(&self) -> Error {
        self.control.abort();
        let stats = self.media.stats(self.negotiation.offers());

        Error::NoMedia {
            datagrams: stats.datagrams,
            authenticated: stats.authenticated,
        }
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending().await,
    }
}
