/// An error that stops the connection from going further.
///
/// - Bytes from the server that don't parse produce `Truncated`, `Malformed`, or `TooLong`, and the connection must be dropped.
/// - `Session` wraps errors in `Failed` to report which connection stage failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("a network error occurred")]
    Io(#[from] std::io::Error),
    #[error("no IPv4 address found for {0}")]
    Resolve(String),
    #[error("the server timed out")]
    Timeout,
    #[error("the server sent a truncated {0}")]
    Truncated(&'static str),
    #[error("the server sent a malformed {0}")]
    Malformed(&'static str),
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("{what} length {len} exceeds the limit")]
    TooLong { what: &'static str, len: usize },
    #[error("the server does not support RSA-SRP authentication")]
    Unsupported,
    #[error("the server's public key differs from the previous connection")]
    KeyChanged,
    #[error("incorrect username or password")]
    Rejected,
    #[error("another user ({0}) is logged in to the Mac")]
    OtherUser(String),
    #[error("the control channel failed its integrity check")]
    Integrity,
    #[error("the server changed keys during the session")]
    Rekey,
    /// The server never started sending video; the two counts tell the causes apart.
    ///
    /// - Zero `datagrams` means no UDP arrived (firewall, port, NAT); zero `authenticated` means the SRTP keys or port pairing don't match.
    #[error(
        "the server never started sending video ({datagrams} datagrams received, {authenticated} passed SRTP authentication)"
    )]
    NoMedia { datagrams: u64, authenticated: u64 },
    #[error("the session has already ended")]
    Closed,
    #[error("connection failed at the {stage} stage")]
    Failed {
        stage: crate::Stage,
        #[source]
        source: Box<Error>,
    },
}

/// `Result` with the core's error as the default error type.
pub type Result<T, E = Error> = std::result::Result<T, E>;
