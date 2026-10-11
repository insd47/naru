//! 세션을 Kotlin과 Swift에 노출하는 UniFFI 경계입니다.
//!
//! - 네이티브 쪽은 [`Session::connect`]에 [`SessionListener`]를 넘겨 세션을 열고, 사건은 세션 전용 스레드에서 차례로 받습니다.
//! - 코어 타입 가운데 튜플이나 내부 필드가 없는 것은 그대로 넘기고, 나머지는 여기서 네이티브 쪽에 맞는 모양으로 옮깁니다.
use crate::{
    AccessUnit, AudioFrame, DecoderConfig, Error, InputSource, Language, MediaStats, Mode, Phase, Priority, Stage,
};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, LazyLock};
use tokio::runtime::Runtime;

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
    /// 소리를 받을지 여부. AAC-ELD 디코더가 없는 기기에서는 끕니다.
    pub audio: bool,
    /// 패킷을 잃었을 때 지연과 화질 중 무엇을 우선할지
    pub priority: Priority,
}

impl From<Config> for crate::Config {
    fn from(config: Config) -> Self {
        let mut core = Self::new(config.host, config.username, config.password);
        core.port = config.port;
        core.known_key = config.known_key;
        core.display = config.display.map(Into::into);
        core.audio = config.audio;
        core.priority = config.priority;
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
    // 변형 이름이 실어 나르는 타입과 같으면 생성된 Kotlin에서 중첩 클래스가 그 타입을 가리므로, 이 둘은 이름을 다르게 둡니다.
    /// 새 디코더 설정. 디코더를 이 설정으로 다시 만듭니다.
    Decoder {
        config: DecoderConfig,
    },
    /// 오디오 디코더 설정(AudioSpecificConfig). 첫 오디오 앞에 한 번 옵니다.
    AudioConfig {
        config: Vec<u8>,
    },
    /// 오디오 접근 단위
    Audio {
        frame: AudioFrame,
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
    /// Mac의 현재 입력 소스. 글자 변환은 세션이 하므로 기록과 표시에만 씁니다.
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

/// 세션의 사건을 받는 쪽입니다.
#[uniffi::export(with_foreign)]
pub trait SessionListener: Send + Sync {
    /// 사건 하나. 세션 전용 스레드에서 차례로 부르며, 마지막 사건은 항상 `Closed`입니다.
    ///
    /// - 반환할 때까지 다음 사건이 기다리고, 밀리면 세션도 기다립니다. 오래 걸리는 일은 다른 스레드로 넘기십시오.
    fn on_event(&self, event: Event);
}

/// 실행 중인 세션. 끝난 뒤에 부른 메서드는 아무 일도 하지 않습니다.
#[derive(uniffi::Object)]
pub struct Session {
    session: crate::Session,
}

#[uniffi::export]
impl Session {
    /// 접속을 시작하고 곧바로 반환합니다. 진행 상황은 `listener`로 옵니다.
    #[uniffi::constructor]
    pub fn connect(config: Config, listener: Arc<dyn SessionListener>) -> Arc<Self> {
        let _runtime = RUNTIME.enter();
        let (session, mut events) = crate::Session::start(config.into());

        // 사건마다 네이티브 코드를 부르므로 tokio 작업 스레드를 붙잡지 않게 전용 스레드에서 넘깁니다.
        std::thread::Builder::new()
            .name("naru-events".into())
            .spawn(move || {
                // 네이티브 쪽이 예외를 던지면 UniFFI가 패닉으로 바꿉니다. 그 사건만 버리고 계속 넘겨야 마지막 `Closed`가 닿습니다.
                while let Some(event) = events.blocking_recv() {
                    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| listener.on_event(event.into())));
                }
            })
            .expect("사건 스레드를 만들 수 없습니다");

        Arc::new(Self { session })
    }

    /// 키 입력. `keysym`은 X11 keysym이고, 글자 키는 물리 키의 라틴 문자로 보냅니다. 한국어 자모로는 세션이 바꿉니다.
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

    /// 키프레임을 요청합니다. 디코더를 새로 만들었거나 입력이 밀려 프레임을 버렸을 때 부릅니다.
    pub fn request_keyframe(&self) {
        let _ = self.session.request_keyframe();
    }

    /// 가상 디스플레이의 크기를 바꿉니다. 새 레이아웃과 디코더 설정이 뒤따릅니다.
    pub fn resize(&self, mode: DisplayMode) {
        let _ = self.session.resize(&mode.into());
    }

    /// 키보드 언어를 고릅니다. 태블릿의 입력 언어가 바뀌면 부릅니다.
    pub fn language(&self, language: Language) {
        let _ = self.session.language(language);
    }

    /// 세션을 닫습니다. Kotlin 객체의 `close`(네이티브 자원 해제)와 이름이 겹치지 않게 따로 둡니다.
    pub fn disconnect(&self) {
        let _ = self.session.close();
    }
}
