//! 세션을 Kotlin과 Swift에 노출하는 UniFFI 경계입니다.
//!
//! - 네이티브 쪽은 [`NaruSession::connect`]로 세션을 열고, 한 스레드에서 [`NaruSession::next_event`]를 되풀이해 사건을 꺼냅니다.
//! - 꺼내는 쪽이 시간 제한을 두므로, 화면이 멈춰 프레임이 오지 않는 동안에도 디코더 출력을 꺼낼 수 있습니다.
//! - 세션은 이 크레이트가 가진 tokio 런타임에서 돌아갑니다. 네이티브 쪽에 런타임이 따로 필요하지 않습니다.
use crate::media::AccessUnit;
use crate::media::DecoderConfig;
use crate::message::gesture;
use crate::message::{CursorImage, DisplayConfig, Layout, Mode};
use crate::{Config, Event, Session};
use std::error::Error as _;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

// 세션 하나는 task 하나로 돌므로 작업 스레드는 둘이면 충분합니다.
static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("naru-protocol")
        .enable_all()
        .build()
        .expect("tokio 런타임을 만들 수 없습니다")
});

/// 접속 설정
#[derive(uniffi::Record)]
pub struct ConnectConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// 이전 접속에서 저장한 서버 공개키(DER). 다르면 접속을 멈춥니다.
    pub known_key: Option<Vec<u8>>,
    /// 가상 디스플레이. 없으면 Mac의 실제 화면을 받습니다.
    pub display: Option<DisplayMode>,
}

/// 가상 디스플레이의 논리 크기와 배율. 픽셀 크기는 논리 크기 × 배율입니다.
#[derive(uniffi::Record)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

/// 접속 단계
#[derive(uniffi::Enum)]
pub enum Stage {
    Warmup,
    Handshake,
    Auth,
    KeyExchange,
    Offer,
    Media,
}

/// 스크롤과 확대 제스처의 단계
#[derive(uniffi::Enum)]
pub enum Phase {
    Began,
    Changed,
    Ended,
    Cancelled,
}

/// 세션이 알리는 사건
#[derive(uniffi::Enum)]
pub enum SessionEvent {
    Progress {
        stage: Stage,
    },
    /// 서버 공개키(DER). 처음 접속했다면 저장해 두고 다음 접속의 `known_key`로 넘깁니다.
    ServerKey {
        der: Vec<u8>,
    },
    Authenticated {
        server_proof: bool,
    },
    /// 디스플레이 레이아웃. 포인터 좌표는 픽셀 크기를 기준으로 보냅니다.
    Layout {
        logical_width: u16,
        logical_height: u16,
        pixel_width: u16,
        pixel_height: u16,
    },
    /// 새 디코더 설정. 디코더를 이 설정으로 다시 만듭니다. NAL에는 시작 코드가 없습니다.
    DecoderConfig {
        sps: Vec<Vec<u8>>,
        pps: Vec<Vec<u8>>,
    },
    /// 디코더에 넣을 접근 단위. NAL에는 시작 코드가 없습니다.
    Frame {
        keyframe: bool,
        /// RTP 타임스탬프(90 kHz)
        timestamp: u32,
        nals: Vec<Vec<u8>>,
    },
    /// 커서 모양. 행 우선 RGBA, 알파는 미리 곱하지 않은 값입니다.
    Cursor {
        width: u16,
        height: u16,
        hotspot_x: u16,
        hotspot_y: u16,
        rgba: Vec<u8>,
    },
    /// Mac의 컴퓨터 이름. 인증 뒤에 한 번 옵니다.
    ServerName {
        name: String,
    },
    InputSource {
        id: String,
    },
    Clipboard {
        text: String,
    },
    /// 프레임이 오래 나오지 않았습니다. 디코더를 다시 만듭니다. 키프레임은 세션이 요청합니다.
    ResetDecoder,
    /// 미디어 수신 통계. 미디어 협상이 끝난 뒤 0.5초마다 옵니다.
    Stats {
        datagrams: u64,
        authenticated: u64,
        ssrc: Option<u32>,
        highest: Option<u32>,
        candidates: u32,
        offers: u32,
        nacked: u64,
        filled: u64,
        losses: u64,
    },
    /// 세션이 끝났습니다. 닫아서 끝났으면 둘 다 없습니다.
    Closed {
        failure: Option<Failure>,
        /// 원인을 이은 한 줄. 로그용입니다.
        detail: Option<String>,
    },
}

/// 접속이 끝난 이유. 사용자에게 보여 줄 문구와 다음 행동을 고르는 데 씁니다.
#[derive(uniffi::Enum)]
pub enum Failure {
    /// 사용자 이름이나 비밀번호가 틀렸습니다.
    Rejected,
    /// 서버의 공개키가 저장한 키와 다릅니다.
    KeyChanged,
    /// 다른 사용자가 Mac에 로그인해 있습니다.
    OtherUser { name: String },
    /// 주소를 풀지 못했거나 연결하지 못했습니다.
    Unreachable,
    /// 서버가 이 클라이언트의 인증 방식을 지원하지 않습니다.
    Unsupported,
    /// 서버가 영상을 보내기 시작하지 않았습니다.
    NoMedia,
    /// 그 밖의 오류
    Other,
}

/// 실행 중인 세션
#[derive(uniffi::Object)]
pub struct NaruSession {
    session: Session,
    events: Mutex<mpsc::Receiver<Event>>,
}

#[uniffi::export]
impl NaruSession {
    /// 접속을 시작하고 곧바로 반환합니다. 진행 상황은 [`Self::next_event`]로 받습니다.
    #[uniffi::constructor]
    pub fn connect(config: ConnectConfig) -> Arc<Self> {
        let mut core = Config::new(config.host, config.username, config.password);
        core.port = config.port;
        core.known_key = config.known_key;
        core.display = config.display.map(display_config);

        let _runtime = RUNTIME.enter();
        let (session, events) = Session::start(core);

        Arc::new(Self {
            session,
            events: Mutex::new(events),
        })
    }

    /// 다음 사건을 기다립니다. `timeout_ms` 안에 오지 않았거나 세션이 이미 끝났으면 `None`입니다.
    ///
    /// - 한 스레드에서만 부르십시오. 마지막 사건은 항상 `Closed`입니다.
    pub fn next_event(&self, timeout_ms: u32) -> Option<SessionEvent> {
        let mut events = self.events.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let timeout = Duration::from_millis(timeout_ms.into());

        RUNTIME.block_on(async {
            let deadline = tokio::time::Instant::now() + timeout;

            // 네이티브 쪽이 쓰지 않는 사건은 건너뜁니다.
            loop {
                let event = tokio::time::timeout_at(deadline, events.recv()).await.ok()??;

                if let Some(event) = convert(event) {
                    return Some(event);
                }
            }
        })
    }

    /// 키 입력. `keysym`은 X11 keysym입니다.
    pub fn key(&self, keysym: u32, down: bool) {
        let _ = self.session.key(keysym, down);
    }

    /// 포인터 위치(레이아웃의 픽셀 좌표)와 눌린 버튼(왼쪽 1, 오른쪽 2, 가운데 4)
    pub fn pointer(&self, buttons: u8, x: u16, y: u16) {
        let _ = self.session.pointer(buttons, x, y);
    }

    /// 트랙패드 스크롤. 이동량은 포인트이고 방향은 양수가 위, 왼쪽입니다. 스크롤은 마지막 포인터 위치에서 일어납니다.
    ///
    /// - 손을 떼면 `Ended`를 보냅니다. 관성은 세션이 이어 보냅니다.
    pub fn scroll(&self, phase: Phase, dx: f32, dy: f32) {
        let _ = self.session.scroll(gesture_phase(phase), (dx, dy));
    }

    /// 마우스 휠. 칸 수이며 방향은 스크롤과 같습니다.
    pub fn wheel(&self, dx: f32, dy: f32) {
        let _ = self.session.wheel((dx, dy));
    }

    /// 확대 제스처. 배율은 시작할 때 1.0이고 누적됩니다.
    pub fn magnify(&self, phase: Phase, scale: f32) {
        let _ = self.session.magnify(gesture_phase(phase), scale);
    }

    pub fn clipboard(&self, text: String) {
        let _ = self.session.clipboard(text);
    }

    /// 디코더 오류 뒤 키프레임을 요청합니다.
    pub fn request_keyframe(&self) {
        let _ = self.session.request_keyframe();
    }

    /// 가상 디스플레이의 크기를 바꿉니다. 새 레이아웃과 디코더 설정이 뒤따릅니다.
    pub fn resize(&self, mode: DisplayMode) {
        let _ = self.session.resize(&display_config(mode));
    }

    /// Mac의 입력 소스를 지정합니다. 바뀌면 `InputSource` 사건이 옵니다.
    pub fn input_source(&self, id: String) {
        let _ = self.session.input_source(&id);
    }

    /// 세션을 닫습니다. Kotlin 객체의 `close`(네이티브 자원 해제)와 이름이 겹치지 않게 따로 둡니다.
    pub fn disconnect(&self) {
        let _ = self.session.close();
    }
}

// 기기 이름과 실제 크기는 서버의 디스플레이 설정 화면에만 쓰입니다.
fn display_config(mode: DisplayMode) -> DisplayConfig {
    DisplayConfig {
        name: "Naru".into(),
        physical_mm: (369.45, 207.82),
        modes: vec![Mode::new((mode.width, mode.height), mode.scale)],
    }
}

fn gesture_phase(phase: Phase) -> gesture::Phase {
    match phase {
        Phase::Began => gesture::Phase::Began,
        Phase::Changed => gesture::Phase::Changed,
        Phase::Ended => gesture::Phase::Ended,
        Phase::Cancelled => gesture::Phase::Cancelled,
    }
}

fn convert(event: Event) -> Option<SessionEvent> {
    Some(match event {
        Event::Progress(stage) => SessionEvent::Progress {
            stage: match stage {
                crate::Stage::Warmup => Stage::Warmup,
                crate::Stage::Handshake => Stage::Handshake,
                crate::Stage::Auth => Stage::Auth,
                crate::Stage::KeyExchange => Stage::KeyExchange,
                crate::Stage::Offer => Stage::Offer,
                crate::Stage::Media => Stage::Media,
            },
        },
        Event::ServerKey(der) => SessionEvent::ServerKey { der },
        Event::Authenticated { server_proof } => SessionEvent::Authenticated { server_proof },
        Event::Layout(layout) => layout_event(&layout),
        Event::DecoderConfig(DecoderConfig { sps, pps, .. }) => SessionEvent::DecoderConfig { sps, pps },
        Event::Frame(AccessUnit {
            keyframe,
            timestamp,
            nals,
            ..
        }) => SessionEvent::Frame {
            keyframe,
            timestamp,
            nals,
        },
        Event::Cursor(image) => cursor_event(&image),
        Event::ServerName(name) => SessionEvent::ServerName { name },
        Event::InputSource(id) => SessionEvent::InputSource { id },
        Event::Clipboard(text) => SessionEvent::Clipboard { text },
        Event::ResetDecoder => SessionEvent::ResetDecoder,
        Event::Stats(stats) => SessionEvent::Stats {
            datagrams: stats.datagrams,
            authenticated: stats.authenticated,
            ssrc: stats.ssrc,
            highest: stats.highest,
            candidates: u32::try_from(stats.candidates).unwrap_or(u32::MAX),
            offers: stats.offers,
            nacked: stats.nacked,
            filled: stats.filled,
            losses: stats.losses,
        },
        Event::Closed(error) => SessionEvent::Closed {
            failure: error.as_ref().map(failure),
            detail: error.as_ref().map(describe),
        },
        _ => return None,
    })
}

fn layout_event(layout: &Layout) -> SessionEvent {
    SessionEvent::Layout {
        logical_width: layout.logical.0,
        logical_height: layout.logical.1,
        pixel_width: layout.pixels.0,
        pixel_height: layout.pixels.1,
    }
}

fn cursor_event(image: &CursorImage) -> SessionEvent {
    SessionEvent::Cursor {
        width: image.width,
        height: image.height,
        hotspot_x: image.hotspot.0,
        hotspot_y: image.hotspot.1,
        rgba: image.rgba.clone(),
    }
}

fn failure(error: &crate::Error) -> Failure {
    use crate::Error;

    match error {
        Error::Failed { source, .. } => failure(source),
        Error::Rejected => Failure::Rejected,
        Error::KeyChanged => Failure::KeyChanged,
        Error::OtherUser(name) => Failure::OtherUser { name: name.clone() },
        Error::Io(_) | Error::Resolve(_) | Error::Timeout => Failure::Unreachable,
        Error::Unsupported => Failure::Unsupported,
        Error::NoMedia { .. } => Failure::NoMedia,
        _ => Failure::Other,
    }
}

// 에러와 원인을 이어 로그에 남길 한 줄로 만듭니다.
fn describe(error: &crate::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();

    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }

    message
}
