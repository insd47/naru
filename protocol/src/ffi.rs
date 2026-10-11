//! 세션을 Kotlin과 Swift에 노출하는 UniFFI 경계입니다.
//!
//! - 네이티브 쪽은 [`Session::connect`]로 세션을 열고, 한 스레드에서 [`Session::next_event`]를 되풀이해 사건을 꺼냅니다.
//! - 꺼내는 쪽이 시간 제한을 두므로, 화면이 멈춰 프레임이 오지 않는 동안에도 디코더 출력을 꺼낼 수 있습니다.
//! - 코어 타입 가운데 튜플이나 내부 필드가 없는 것은 그대로 넘기고, 나머지는 여기서 네이티브 쪽에 맞는 모양으로 옮깁니다.
use crate::{AccessUnit, DecoderConfig, Error, InputSource, MediaStats, Mode, Phase, Stage};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

// 세션은 이 크레이트의 tokio 런타임에서 돌아가므로 네이티브 쪽에 런타임이 따로 필요하지 않습니다.
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
pub struct Config {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// 이전 접속에서 저장한 서버 공개키(DER). 다르면 접속을 멈춥니다.
    pub known_key: Option<Vec<u8>>,
    /// 가상 디스플레이. 없으면 Mac의 실제 화면을 받습니다.
    pub display: Option<DisplayMode>,
}

impl From<Config> for crate::Config {
    fn from(config: Config) -> Self {
        let mut core = Self::new(config.host, config.username, config.password);
        core.port = config.port;
        core.known_key = config.known_key;
        core.display = config.display.map(Into::into);
        core
    }
}

/// 가상 디스플레이의 논리 크기와 배율. 픽셀 크기는 논리 크기 × 배율입니다.
#[derive(uniffi::Record)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

impl From<DisplayMode> for crate::DisplayConfig {
    fn from(mode: DisplayMode) -> Self {
        Mode::new((mode.width, mode.height), mode.scale).into()
    }
}

/// 세션이 알리는 사건
#[derive(uniffi::Enum)]
pub enum Event {
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
    /// Mac의 컴퓨터 이름. 인증 뒤에 한 번 옵니다.
    ServerName {
        name: String,
    },
    /// 디스플레이 레이아웃. 포인터 좌표는 픽셀 크기를 기준으로 보냅니다.
    Layout {
        logical_width: u16,
        logical_height: u16,
        pixel_width: u16,
        pixel_height: u16,
    },
    /// 새 디코더 설정. 디코더를 이 설정으로 다시 만듭니다.
    Decoder {
        config: DecoderConfig,
    },
    /// 디코더에 넣을 접근 단위
    Frame {
        unit: AccessUnit,
    },
    /// 커서 모양. 행 우선 RGBA, 알파는 미리 곱하지 않은 값입니다. 위치는 오지 않습니다.
    Cursor {
        width: u16,
        height: u16,
        hotspot_x: u16,
        hotspot_y: u16,
        rgba: Vec<u8>,
    },
    /// Mac의 현재 입력 소스
    Source {
        source: InputSource,
    },
    Clipboard {
        text: String,
    },
    /// 프레임이 오래 나오지 않았습니다. 디코더를 다시 만듭니다. 키프레임은 세션이 요청합니다.
    ResetDecoder,
    /// 미디어 수신 통계. 미디어 협상이 끝난 뒤 0.5초마다 옵니다.
    Stats {
        stats: MediaStats,
    },
    /// 세션이 끝났습니다. 닫아서 끝났으면 둘 다 없습니다.
    Closed {
        failure: Option<Failure>,
        /// 원인을 이은 한 줄. 로그용입니다.
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
            crate::Event::Frame(unit) => Self::Frame { unit },
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

/// 실행 중인 세션
#[derive(uniffi::Object)]
pub struct Session {
    session: crate::Session,
    events: Mutex<mpsc::Receiver<crate::Event>>,
}

#[uniffi::export]
impl Session {
    /// 접속을 시작하고 곧바로 반환합니다. 진행 상황은 [`Self::next_event`]로 받습니다.
    #[uniffi::constructor]
    pub fn connect(config: Config) -> Arc<Self> {
        let _runtime = RUNTIME.enter();
        let (session, events) = crate::Session::start(config.into());

        Arc::new(Self {
            session,
            events: Mutex::new(events),
        })
    }

    /// 다음 사건을 기다립니다. `timeout_ms` 안에 오지 않았거나 세션이 이미 끝났으면 `None`입니다.
    ///
    /// - 한 스레드에서만 부르십시오. 마지막 사건은 항상 `Closed`입니다.
    pub fn next_event(&self, timeout_ms: u32) -> Option<Event> {
        let mut events = self.events.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let timeout = Duration::from_millis(timeout_ms.into());
        let event = RUNTIME.block_on(tokio::time::timeout(timeout, events.recv()));
        event.ok().flatten().map(Event::from)
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
        let _ = self.session.scroll(phase, (dx, dy));
    }

    /// 마우스 휠. 칸 수이며 방향은 스크롤과 같습니다.
    pub fn wheel(&self, dx: f32, dy: f32) {
        let _ = self.session.wheel((dx, dy));
    }

    /// 확대 제스처. 배율은 시작할 때 1.0이고 누적됩니다.
    pub fn magnify(&self, phase: Phase, scale: f32) {
        let _ = self.session.magnify(phase, scale);
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
        let _ = self.session.resize(&mode.into());
    }

    /// Mac의 입력 소스를 지정합니다. 바뀌면 `Source` 사건이 옵니다.
    pub fn input_source(&self, id: String) {
        let _ = self.session.input_source(&id);
    }

    /// 세션을 닫습니다. Kotlin 객체의 `close`(네이티브 자원 해제)와 이름이 겹치지 않게 따로 둡니다.
    pub fn disconnect(&self) {
        let _ = self.session.close();
    }
}
