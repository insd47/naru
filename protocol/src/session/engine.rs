use super::control::Control;
use super::input::Input;
use super::keyboard::Keyboard;
use super::media::{Media, Sockets};
use super::{Command, Config, Event, Events, Stage};
use crate::error::{Error, Result};
use crate::message::{Cursors, InputSource, Layout, Messages, Rect, ServerMessage, client, clipboard};
use crate::offer::{Answer, Offer};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

const TICK: Duration = Duration::from_millis(500);

// 응답에 크기가 없으면 서버의 인코더가 아직 준비되지 않은 것이므로 같은 오퍼를 다시 보냅니다.
// 응답이 오기 전에는 다시 보내지 않습니다. 서버는 오퍼마다 스트림을 만들므로, 성급히 보내면 스트림이 여럿 생겨 시작이 끊깁니다.
const OFFER_RETRY: Duration = Duration::from_millis(200);
const OFFER_ATTEMPTS: u32 = 16;

// 응답이 아예 없을 때 같은 오퍼를 다시 보내기까지 기다리는 시간입니다. 실측한 응답 시간은 1.5초 안팎입니다.
const OFFER_TIMEOUT: Duration = Duration::from_secs(4);

// 응답을 받은 뒤 이 시간 안에 디코더 설정이 오지 않으면 서버의 인코더가 멈춘 것으로 보고 다시 접속합니다.
const MEDIA_START: Duration = Duration::from_secs(3);

// 프레임이 나오지 않을 때의 조치입니다. 화면이 정지하면 서버가 영상을 보내지 않으므로, 최근에 패킷이 온 경우에만 조치합니다.
const STALL_KEYFRAME: Duration = Duration::from_secs(3);
const STALL_KEYFRAME_REPEAT: Duration = Duration::from_millis(1500);
const STALL_RESETS: [Duration; 2] = [Duration::from_secs(8), Duration::from_secs(15)];

/// 레코드 계층이 켜진 뒤의 세션 상태입니다. 한 task에서 제어 채널, UDP, 타이머, 명령을 모두 처리합니다.
///
/// - 레코드의 순번과 CBC 상태는 보낸 순서에 묶이므로, 제어 채널로 보내는 일은 이 task에서만 합니다.
pub struct Engine {
    control: Control,
    sockets: Sockets,
    media: Media,
    input: Input,
    keyboard: Keyboard,
    offer: Offer,
    messages: Messages,
    cursors: Cursors,
    layout: Option<Layout>,
    offer_sent: (Instant, u32),
    // 크기 없는 응답을 받아 다시 보내기로 한 시각
    offer_retry: Option<Instant>,
    answered: Option<Instant>,
    stall: Stall,
}

// 프레임 정지 감시 상태입니다. 프레임이 나오면 초기화합니다.
#[derive(Default)]
struct Stall {
    last_request: Option<Instant>,
    resets: usize,
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
    Confirm,
    Command(Option<Command>),
}

impl Engine {
    pub fn new(control: Control, sockets: Sockets, config: &Config) -> Self {
        let offer = Offer::new(config.audio);

        Self {
            media: Media::new(sockets.clone(), &offer),
            input: Input::new(control.keys().clone()),
            keyboard: Keyboard::new(),
            control,
            sockets,
            offer,
            messages: Messages::default(),
            cursors: Cursors::default(),
            layout: None,
            offer_sent: (Instant::now(), 0),
            offer_retry: None,
            answered: None,
            stall: Stall::default(),
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
            let confirm = self.keyboard.deadline();
            let retry = self
                .answered
                .is_none()
                .then(|| self.offer_retry.unwrap_or(self.offer_sent.0 + OFFER_TIMEOUT));

            let wake = tokio::select! {
                opened = self.control.receive(&mut tcp_buffer) => Wake::Tcp(opened),
                received = video.recv_from(&mut video_buffer) => Wake::Video(received),
                received = audio.recv_from(&mut audio_buffer) => Wake::Audio(received),
                _ = ticker.tick() => Wake::Tick,
                _ = sleep_until(retry) => Wake::Retry,
                _ = sleep_until(deadline) => Wake::Deadline,
                _ = sleep_until(momentum) => Wake::Momentum,
                _ = sleep_until(confirm) => Wake::Confirm,
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
                Wake::Confirm => self.control.send_all(self.keyboard.expire(now)).await?,
                Wake::Command(None | Some(Command::Close)) => return Ok(()),
                Wake::Command(Some(command)) => self.command(command, now).await?,
            }

            if outgoing.iter().any(|event| matches!(event, Event::Frame(_))) {
                self.stall = Stall::default();
            }

            events.publish(&mut outgoing).await?;
        }
    }

    // 레코드 계층이 켜진 직후의 요청입니다. 오퍼를 보낸 뒤 전체 영역을 비증분으로 요청해 TCP 쪽 송신을 깨웁니다.
    async fn start(&mut self, outgoing: &mut Vec<Event>) -> Result<()> {
        self.control.send(&client::set_encodings()).await?;
        self.control.send(&client::AUTO_CLIPBOARD).await?;
        self.send_offer(Instant::now()).await?;
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

                    // 픽셀 크기가 바뀌면 오퍼를 다시 보내야 서버가 새 크기로 인코딩을 시작합니다.
                    let resized = self
                        .layout
                        .as_ref()
                        .is_some_and(|previous| previous.pixels != layout.pixels);
                    self.input.resize(layout.pixels);
                    self.layout = Some(layout.clone());
                    outgoing.push(Event::Layout(layout));

                    // 크기가 같은 레이아웃(로그인, 잠금, 세션 전환)에서도 요청하지 않으면 커서 모양이 굳습니다.
                    self.control
                        .send(&client::framebuffer_request(false, u16::MAX, u16::MAX))
                        .await?;

                    if resized && self.answered.is_some() {
                        self.control.send(&self.offer.encode()).await?;
                    }
                }
                Rect::MediaStream(body) | Rect::Unknown { rest: body, .. } => self.answer(&body, now, outgoing),
                Rect::InputSource(body) => {
                    let Some(source) = InputSource::parse(&body) else {
                        continue;
                    };

                    outgoing.push(Event::InputSource(source.clone()));
                    self.control.send_all(self.keyboard.observe(source, now)).await?;
                }
                Rect::Skipped(_) => {}
            }
        }

        Ok(())
    }

    // 크기가 있는 미디어 응답이 오면 미디어 수신 단계로 넘어갑니다.
    fn answer(&mut self, body: &[u8], now: Instant, outgoing: &mut Vec<Event>) {
        let answer = Answer::find(body);

        let Some(answer) = answer.filter(|_| self.answered.is_none()) else {
            return;
        };

        if answer.size.is_some() {
            self.answered = Some(now);
            outgoing.push(Event::Progress(Stage::Media));
        } else if self.offer_retry.is_none() {
            self.offer_retry = Some(now + OFFER_RETRY);
        }
    }

    async fn tick(&mut self, now: Instant, outgoing: &mut Vec<Event>) -> Result<()> {
        self.media.tick(now).await;

        let Some(answered) = self.answered else {
            return Ok(());
        };

        outgoing.push(Event::Stats(self.media.stats(self.offer_sent.1)));

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

        self.watch_stall(now, outgoing).await;
        Ok(())
    }

    async fn watch_stall(&mut self, now: Instant, outgoing: &mut Vec<Event>) {
        let since = self.media.last_frame().or(self.answered);

        let Some(stalled) = since
            .map(|since| now.duration_since(since))
            .filter(|_| self.media.active(now))
        else {
            return;
        };

        if let Some(&threshold) = STALL_RESETS.get(self.stall.resets)
            && stalled >= threshold
        {
            self.stall.resets += 1;
            outgoing.push(Event::ResetDecoder);
            self.stall.last_request = None;
        }

        let due = self
            .stall
            .last_request
            .is_none_or(|last| now.duration_since(last) >= STALL_KEYFRAME_REPEAT);

        if stalled >= STALL_KEYFRAME && due {
            self.stall.last_request = Some(now);
            self.media.request_keyframe(now).await;
        }
    }

    async fn command(&mut self, command: Command, now: Instant) -> Result<()> {
        match command {
            Command::Key { keysym, down } => self.control.send_all(self.keyboard.key(keysym, down)).await,
            Command::Language(language) => self.control.send_all(self.keyboard.select(language, now)).await,
            Command::Input(event, at) => self.control.send_all(self.input.handle(event, at)).await,
            Command::Clipboard(text) => self.control.send(&clipboard::message(&text)).await,
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
            Command::Close => Ok(()),
        }
    }

    // 크기가 있는 응답을 받을 때까지 같은 오퍼를 다시 보냅니다.
    async fn retry(&mut self, now: Instant) -> Result<()> {
        if self.offer_sent.1 >= OFFER_ATTEMPTS {
            return Err(self.abort());
        }

        self.send_offer(now).await
    }

    async fn send_offer(&mut self, now: Instant) -> Result<()> {
        self.offer_sent = (now, self.offer_sent.1 + 1);
        self.offer_retry = None;
        self.control.send(&self.offer.encode()).await
    }

    // 미디어가 시작되지 않으면 연결을 즉시 끊습니다(RST). 서버 쪽 세션이 남지 않게 해야 다시 접속할 수 있습니다.
    fn abort(&self) -> Error {
        self.control.abort();
        let stats = self.media.stats(self.offer_sent.1);

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
