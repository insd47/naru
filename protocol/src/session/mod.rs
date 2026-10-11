//! 서버 하나와의 세션입니다. 소켓을 쥐고 접속, 인증, 미디어 협상, 수신, 재접속을 처리합니다.
//!
//! - `Session::start`는 곧바로 반환하고, 진행 상황과 결과는 이벤트로 알립니다. 마지막 이벤트는 항상 `Closed`입니다.
//! - 네이티브 계층은 완성된 접근 단위와 디코더 설정만 받습니다. 디코더 오류는 `request_keyframe`로 알려 주십시오.
//! - 미디어가 시작되지 않으면 연결을 끊고 처음부터 다시 접속합니다. 접속 시도는 합쳐서 세 번까지입니다.
use crate::error::{Error, Result};
use crate::media::AccessUnit;
use crate::media::h264::DecoderConfig;
use crate::message::{CursorImage, DisplayConfig, Layout};
use control::Control;
use engine::Engine;
use media::Sockets;
use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

mod control;
mod engine;
mod media;

const ATTEMPTS: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_millis(800);

// 이벤트를 받는 쪽이 느리면 세션이 기다립니다. 영상 프레임이 쌓여 메모리가 늘어나지 않게 합니다.
const EVENT_CAPACITY: usize = 256;

/// 접속 설정입니다.
pub struct Config {
    /// 호스트 이름 또는 주소. IPv4로 풀어 TCP와 UDP에 같은 주소를 씁니다.
    pub host: String,
    /// 서버의 TCP 포트. 오디오는 같은 번호, 비디오는 다음 번호의 UDP 포트로 옵니다.
    pub port: u16,
    pub username: String,
    pub password: String,
    /// 이전 접속에서 저장한 서버 공개키(DER). 다르면 `KeyChanged`로 접속을 멈춥니다.
    pub known_key: Option<Vec<u8>>,
    /// 가상 디스플레이 설정. 보내면 Mac의 실제 화면이 가려지고, 보내지 않으면 실제 화면을 그대로 받습니다.
    pub display: Option<DisplayConfig>,
    /// 오디오 수신 여부. 디코더가 준비되기 전까지는 끕니다.
    pub audio: bool,
    /// 로컬 UDP 포트(오디오, 비디오). 기본값은 서버와 같은 번호입니다.
    pub local_ports: Option<(u16, u16)>,
}

impl Config {
    /// 기본 포트(5900)에 오디오와 가상 디스플레이 없이 접속하는 설정을 만듭니다.
    pub fn new(host: impl Into<String>, username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 5900,
            username: username.into(),
            password: password.into(),
            known_key: None,
            display: None,
            audio: false,
            local_ports: None,
        }
    }
}

/// 접속 단계입니다. 실패한 단계를 사용자에게 구분해 보여 주면 원인을 찾기 쉽습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Stage {
    /// 예열 TCP 연결
    Warmup,
    /// RFB 버전과 보안 유형 교환
    Handshake,
    /// RSA-SRP 인증
    Auth,
    /// 평문 프렐류드와 레코드 계층 활성화
    KeyExchange,
    /// 미디어 오퍼 교환
    Offer,
    /// 미디어 수신 대기
    Media,
}

impl fmt::Display for Stage {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(match self {
            Self::Warmup => "예열",
            Self::Handshake => "핸드셰이크",
            Self::Auth => "인증",
            Self::KeyExchange => "키 교체",
            Self::Offer => "미디어 협상",
            Self::Media => "미디어 수신",
        })
    }
}

/// 세션이 알리는 사건입니다.
#[derive(Debug)]
#[non_exhaustive]
pub enum Event {
    /// 이 단계를 시작했습니다.
    Progress(Stage),
    /// 서버의 공개키(DER). 처음 접속했다면 저장해 두고 다음 접속의 `known_key`로 넘깁니다.
    ServerKey(Vec<u8>),
    /// 인증에 성공했습니다. `server_proof`는 서버의 SRP 증명(M2)을 확인했는지 여부입니다.
    Authenticated { server_proof: bool },
    /// 디스플레이 레이아웃. 포인터 좌표와 디코더 출력의 기준입니다.
    Layout(Layout),
    /// 새 디코더 설정. 디코더를 이 설정으로 다시 만듭니다.
    DecoderConfig(DecoderConfig),
    /// 디코더에 넣을 접근 단위. 디코더 설정 뒤에만 오며, 손실 뒤에는 키프레임부터 옵니다.
    Frame(AccessUnit),
    /// 커서 모양. 포인터 아이콘으로 씁니다. 위치는 오지 않습니다.
    Cursor(Arc<CursorImage>),
    /// Mac의 현재 입력 소스 ID(예: `com.apple.keylayout.ABC`)
    InputSource(String),
    /// Mac의 클립보드 텍스트
    Clipboard(String),
    /// 프레임이 오래 나오지 않아 디코더를 다시 만들어야 합니다. 키프레임은 세션이 요청합니다.
    ResetDecoder,
    /// 세션이 끝났습니다. `close()`로 닫았으면 `None`입니다.
    Closed(Option<Error>),
}

/// 네이티브 계층이 세션에 보내는 명령입니다.
enum Command {
    Key { keysym: u32, down: bool },
    Pointer { buttons: u8, x: u16, y: u16 },
    Clipboard(String),
    Keyframe,
    Resize(Vec<u8>),
    Close,
}

/// 실행 중인 세션에 입력과 명령을 보냅니다. 복제하면 같은 세션을 가리킵니다.
#[derive(Clone)]
pub struct Session {
    commands: mpsc::UnboundedSender<Command>,
}

impl Session {
    /// 세션을 시작합니다. 현재 tokio 런타임에서 실행되며, 진행 상황은 반환한 수신기로 받습니다.
    pub fn start(config: Config) -> (Self, mpsc::Receiver<Event>) {
        let (commands, receiver) = mpsc::unbounded_channel();
        let (events, events_receiver) = mpsc::channel(EVENT_CAPACITY);

        tokio::spawn(run(config, receiver, events));
        (Self { commands }, events_receiver)
    }

    /// 키를 누르거나 뗍니다. 키는 X11 키심이며, Command는 Super(`0xffeb`), Option은 Alt(`0xffe9`)입니다.
    pub fn key(&self, keysym: u32, down: bool) -> Result<()> {
        self.send(Command::Key { keysym, down })
    }

    /// 포인터 위치와 버튼 상태를 보냅니다. 좌표는 `Layout.pixels` 기준의 픽셀 좌표입니다.
    pub fn pointer(&self, buttons: u8, x: u16, y: u16) -> Result<()> {
        self.send(Command::Pointer { buttons, x, y })
    }

    /// Mac의 클립보드에 텍스트를 넣습니다.
    pub fn clipboard(&self, text: String) -> Result<()> {
        self.send(Command::Clipboard(text))
    }

    /// 키프레임을 요청합니다. 디코더가 오류를 알렸을 때 부릅니다. 키프레임이 오기 전의 프레임은 버립니다.
    pub fn request_keyframe(&self) -> Result<()> {
        self.send(Command::Keyframe)
    }

    /// 가상 디스플레이의 크기를 바꿉니다. 실제로 적용된 크기는 `Event::Layout`으로 옵니다.
    ///
    /// - 모드가 없거나 픽셀 크기가 서버의 한도를 넘으면 보내지 않고 `Invalid`를 반환합니다.
    pub fn resize(&self, display: &DisplayConfig) -> Result<()> {
        self.send(Command::Resize(display.encode()?))
    }

    /// 세션을 닫습니다. `Event::Closed(None)`이 마지막으로 옵니다.
    pub fn close(&self) -> Result<()> {
        self.send(Command::Close)
    }

    fn send(&self, command: Command) -> Result<()> {
        self.commands.send(command).map_err(|_| Error::Closed)
    }
}

async fn run(config: Config, mut commands: mpsc::UnboundedReceiver<Command>, events: mpsc::Sender<Event>) {
    let mut result = Ok(());

    for attempt in 1..=ATTEMPTS {
        result = attempt_session(&config, &mut commands, &events).await;

        let retry = matches!(&result, Err(Error::Failed { source, .. }) if matches!(**source, Error::NoMedia { .. }));

        if !retry || attempt == ATTEMPTS {
            break;
        }

        tokio::time::sleep(RETRY_DELAY).await;
    }

    let _ = events.send(Event::Closed(result.err())).await;
}

async fn attempt_session(
    config: &Config,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    events: &mpsc::Sender<Event>,
) -> Result<()> {
    let progress = |stage| events.send(Event::Progress(stage));

    progress(Stage::Warmup).await.map_err(|_| Error::Closed)?;
    let server = resolve(&config.host, config.port)
        .await
        .map_err(|error| error.at(Stage::Warmup))?;
    let local_ports = config.local_ports.unwrap_or((config.port, config.port + 1));
    let sockets = Sockets::bind(server, config.port, local_ports).map_err(|error| error.at(Stage::Warmup))?;

    // 핸드셰이크 동안 로컬 방화벽이 서버의 미디어를 응답으로 받아들이도록 UDP를 보냅니다. 레코드 계층이 켜지면 멈춥니다.
    let punch = tokio::spawn(sockets.clone().punch());
    let control = Control::connect(config, server, events).await;
    punch.abort();

    Engine::new(control?, sockets, config).run(commands, events).await
}

async fn resolve(host: &str, port: u16) -> Result<IpAddr> {
    // `.local` 이름이 IPv6로만 풀리면 TCP는 붙지만 UDP 미디어가 오지 않으므로 IPv4로 고정합니다.
    let mut addresses = tokio::net::lookup_host((host, port)).await?;
    let address = addresses.find(|address| address.is_ipv4());
    address
        .map(|address| address.ip())
        .ok_or_else(|| Error::Resolve(host.into()))
}
