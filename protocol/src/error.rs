/// 접속을 더 진행할 수 없는 경우의 에러입니다.
///
/// - 서버가 보낸 바이트가 형식에 맞지 않으면 `Truncated`, `Malformed`, `TooLong` 중 하나이며, 연결을 끊어야 합니다.
/// - 접속 과정의 어느 단계에서 실패했는지는 `Session`이 `Failed`로 감싸 알려 줍니다.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("네트워크 오류가 발생했습니다")]
    Io(#[from] std::io::Error),
    #[error("{0}의 IPv4 주소를 찾지 못했습니다")]
    Resolve(String),
    #[error("서버의 응답 시간이 초과되었습니다")]
    Timeout,
    #[error("서버가 보낸 {0}이(가) 잘려 있습니다")]
    Truncated(&'static str),
    #[error("서버가 보낸 {0}의 형식이 올바르지 않습니다")]
    Malformed(&'static str),
    #[error("{0}이(가) 올바르지 않습니다")]
    Invalid(&'static str),
    #[error("{what}의 길이 {len}이(가) 한도를 넘습니다")]
    TooLong { what: &'static str, len: usize },
    #[error("서버가 RSA-SRP 인증을 지원하지 않습니다")]
    Unsupported,
    #[error("서버의 공개키가 이전에 접속했을 때와 다릅니다")]
    KeyChanged,
    #[error("사용자 이름 또는 비밀번호가 올바르지 않습니다")]
    Rejected,
    #[error("다른 사용자({0})가 Mac에 로그인해 있습니다")]
    OtherUser(String),
    #[error("제어 채널의 무결성 검사에 실패했습니다")]
    Integrity,
    #[error("서버가 세션 중에 키를 교체했습니다")]
    Rekey,
    /// 서버가 영상을 보내기 시작하지 않았습니다. 두 수로 원인을 가릅니다.
    ///
    /// - `datagrams`가 0이면 UDP가 오지 않은 것(방화벽, 포트, NAT)이고, `authenticated`가 0이면 SRTP 키나 포트 짝이 맞지 않은 것입니다.
    #[error(
        "서버가 영상을 보내기 시작하지 않았습니다(서버에서 받은 데이터그램 {datagrams}개, 그중 SRTP 인증 통과 {authenticated}개)"
    )]
    NoMedia { datagrams: u64, authenticated: u64 },
    #[error("세션이 이미 종료되었습니다")]
    Closed,
    #[error("{stage} 단계에서 접속에 실패했습니다")]
    Failed {
        stage: crate::Stage,
        #[source]
        source: Box<Error>,
    },
}

/// 코어의 오류를 기본값으로 하는 `Result`입니다.
pub type Result<T, E = Error> = std::result::Result<T, E>;
