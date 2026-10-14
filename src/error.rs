/// An error that stops the connection from going further.
///
/// - Bytes from the server that don't parse produce `Truncated`, `Malformed`, or `TooLong`, and the connection must be
///   dropped.
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
    /// The server never started sending video; the reception stats tell the causes apart.
    ///
    /// - Zero `datagrams` means no UDP arrived (firewall, port, NAT); zero `authenticated` means the SRTP keys or port
    ///   pairing don't match.
    /// - Packets arriving without a start, with losses and a small `largest_bytes`, point at a path that drops large
    ///   packets.
    #[error(
        "the server never started sending video ({} datagrams received, {} passed SRTP authentication, largest {} bytes, {} missing, {} lost, {} other streams)",
        .0.datagrams, .0.authenticated, .0.largest_bytes, .0.missing, .0.losses, .0.candidates
    )]
    NoMedia(crate::MediaStats),
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
