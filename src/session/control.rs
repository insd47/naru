use super::{Config, Event, Events, Stage};
use crate::auth::{Challenge, KEY_REQUEST, MAX_MESSAGE_BYTES, SECURITY_TYPE, ServerKey, VERSION};
use crate::error::{Error, Result};
use crate::message::{Messages, Rect, ServerMessage, client};
use crate::record::{Opener, RecordKeys, Sealer};
use socket2::SockRef;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

// Wait after closing the warm-up connection. With a warm-up, the server does not drop TCP when moving from the lock
// screen to the desktop.
const WARMUP_DELAY: Duration = Duration::from_millis(1400);

// The server needs a moment after ViewerInfo with the encryption start request, and again after the activation request.
const PRELUDE_DELAY: Duration = Duration::from_millis(100);
const ACTIVATE_DELAY: Duration = Duration::from_millis(200);

// When another user is logged in, a session selection prompt follows ServerInit right away.
const PROMPT_WAIT: Duration = Duration::from_millis(300);
const PROMPT_MARKER: [u8; 4] = [0, 0x4a, 0, 1];
const PROMPT_BYTES: usize = 76;

const STEP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_NAME_BYTES: usize = 1 << 10;
const READ_BYTES: usize = 64 << 10;

/// A control channel with the record layer active.
pub struct Control {
    stream: TcpStream,
    keys: RecordKeys,
    sealer: Sealer,
    opener: Opener,
    // Rectangles that came with the key exchange during the plaintext stretch, such as cursor or layout.
    early: Vec<Rect>,
}

impl Control {
    /// Activates the record layer after the warm-up connection, handshake, authentication, and plaintext prelude.
    ///
    /// - TCP connects to the same IPv4 address as UDP. Connecting by host name may put only TCP on IPv6, and then media
    ///   never arrives.
    pub async fn connect(config: &Config, server: IpAddr, events: &mut Events) -> Result<Self> {
        let address = SocketAddr::from((server, config.port));

        warmup(address).await?;

        events.send(Event::Progress(Stage::Handshake)).await?;
        let mut stream = within(TcpStream::connect(address)).await?;
        stream.set_nodelay(true)?;
        handshake(&mut stream).await?;

        events.send(Event::Progress(Stage::Auth)).await?;
        let key = within(server_key(&mut stream)).await?;
        events.send(Event::ServerKey(key.der().to_vec())).await?;

        if config.known_key.as_deref().is_some_and(|known| known != key.der()) {
            return Err(Error::KeyChanged);
        }

        let (wrap_key, server_proof) = within(authenticate(&mut stream, &key, config)).await?;
        events.send(Event::Authenticated { server_proof }).await?;

        events.send(Event::Progress(Stage::KeyExchange)).await?;
        within(prelude(stream, &wrap_key, config, events)).await
    }

    /// Record layer keys, also used to encrypt pointer events
    pub fn keys(&self) -> &RecordKeys {
        &self.keys
    }

    /// What already arrived while connecting: rectangles from the plaintext stretch and bodies of records that came in
    /// the same read right after the key exchange
    pub fn backlog(&mut self) -> Result<(Vec<Rect>, Vec<Vec<u8>>)> {
        Ok((std::mem::take(&mut self.early), self.opened()?))
    }

    /// Returns the bodies of records completed by the next read, or `Closed` when the server closed the connection.
    pub async fn receive(&mut self, buffer: &mut [u8]) -> Result<Vec<Vec<u8>>> {
        let read = self.stream.read(buffer).await?;

        if read == 0 {
            return Err(Error::Closed);
        }

        self.opener.push(&buffer[..read]);
        self.opened()
    }

    /// Makes closing reset the connection immediately (RST). Reconnecting only works once no session lingers on the
    /// server.
    pub fn abort(&self) {
        let _ = SockRef::from(&self.stream).set_linger(Some(Duration::ZERO));
    }

    fn opened(&mut self) -> Result<Vec<Vec<u8>>> {
        std::iter::from_fn(|| self.opener.pop().transpose()).collect()
    }

    /// Sends a message wrapped in records.
    pub async fn send(&mut self, message: &[u8]) -> Result<()> {
        let records = self.sealer.seal(message);
        self.stream.write_all(&records).await?;
        Ok(())
    }
}

async fn warmup(address: SocketAddr) -> Result<()> {
    let mut stream = within(TcpStream::connect(address)).await?;
    let mut version = [0; 12];
    within(stream.read_exact(&mut version)).await?;
    stream.write_all(VERSION).await?;
    within(security_types(&mut stream)).await?;
    drop(stream);

    tokio::time::sleep(WARMUP_DELAY).await;
    Ok(())
}

async fn handshake(stream: &mut TcpStream) -> Result<()> {
    let mut version = [0; 12];
    within(stream.read_exact(&mut version)).await?;

    if !version.starts_with(b"RFB ") {
        return Err(Error::Malformed("RFB version"));
    }

    stream.write_all(VERSION).await?;

    let types = within(security_types(stream)).await?;
    types.contains(&SECURITY_TYPE).then_some(()).ok_or(Error::Unsupported)
}

async fn security_types(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let count = stream.read_u8().await?;

    if count == 0 {
        return Err(Error::Unsupported);
    }

    let mut types = vec![0; usize::from(count)];
    stream.read_exact(&mut types).await?;
    Ok(types)
}

async fn server_key(stream: &mut TcpStream) -> Result<ServerKey> {
    stream.write_all(&KEY_REQUEST).await?;
    ServerKey::parse(&message(stream).await?)
}

// Returns the wrap key and whether the server proof (M2) was found in the result block.
async fn authenticate(stream: &mut TcpStream, key: &ServerKey, config: &Config) -> Result<([u8; 16], bool)> {
    stream.write_all(&key.identity(&config.username)?).await?;

    let challenge = Challenge::parse(&message(stream).await?)?;

    // PBKDF2 takes hundreds of milliseconds, so keep it off the runtime's other tasks.
    let password = config.password.clone();
    let proof = tokio::task::spawn_blocking(move || challenge.respond(&password))
        .await
        .map_err(|_| Error::Closed)?;

    stream.write_all(&proof.packet).await?;

    // The result follows the proof block. The server proof's position is not certain, so a failed check does not block
    // the connection.
    let block = message(stream).await?;

    if stream.read_u32().await? != 0 {
        return Err(Error::Rejected);
    }

    Ok((proof.wrap_key, proof.verify(&block)))
}

async fn prelude(mut stream: TcpStream, wrap_key: &[u8; 16], config: &Config, events: &mut Events) -> Result<Control> {
    stream.write_all(&client::CLIENT_INIT).await?;
    let name = server_init(&mut stream).await?;
    events.send(Event::ServerName(name)).await?;

    let mut messages = Messages::default();
    let pending = session_prompt(&mut stream).await?;
    messages.push(&pending);

    stream
        .write_all(&[client::viewer_info().as_slice(), &client::ENCRYPTION_START].concat())
        .await?;
    tokio::time::sleep(PRELUDE_DELAY).await;

    if let Some(display) = &config.display {
        stream.write_all(&display.encode()?).await?;
    }

    stream.write_all(&client::set_encodings()).await?;

    // Records start where the message with the key exchange rectangle ends, possibly within the same read.
    let mut early = Vec::new();
    let mut buffer = vec![0; READ_BYTES];

    let exchange = loop {
        if let Some(exchange) = key_exchange(&mut messages, &mut early)? {
            break exchange;
        }

        let read = stream.read(&mut buffer).await?;

        if read == 0 {
            return Err(Error::Closed);
        }

        messages.push(&buffer[..read]);
    };

    let keys = RecordKeys::unwrap(&exchange, wrap_key);
    let mut opener = Opener::new(&keys);
    opener.push(&messages.take());

    stream.write_all(&client::ENCRYPTION_ACTIVATE).await?;
    tokio::time::sleep(ACTIVATE_DELAY).await;

    Ok(Control {
        stream,
        sealer: Sealer::new(&keys),
        keys,
        opener,
        early,
    })
}

// Reads ServerInit and returns the Mac's computer name.
async fn server_init(stream: &mut TcpStream) -> Result<String> {
    let mut fixed = [0; 24];
    stream.read_exact(&mut fixed).await?;

    let len = u32::from_be_bytes(fixed[20..24].try_into().expect("4 bytes")) as usize;

    if len > MAX_NAME_BYTES {
        return Err(Error::TooLong {
            what: "server name",
            len,
        });
    }

    let mut name = vec![0; len];
    stream.read_exact(&mut name).await?;
    Ok(computer_name(&name))
}

// The name field has a binary header containing NULs and NUL padding before the computer name. The name follows the
// last NUL.
fn computer_name(bytes: &[u8]) -> String {
    let start = bytes.iter().rposition(|&byte| byte == 0).map_or(0, |at| at + 1);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

// Checks for the session selection prompt after ServerInit, returning the bytes read when it is not one.
async fn session_prompt(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut head = Vec::new();

    while head.len() < PROMPT_MARKER.len() {
        let mut byte = [0; 1];

        match timeout(PROMPT_WAIT, stream.read(&mut byte)).await {
            Ok(Ok(0)) => return Err(Error::Closed),
            Ok(Ok(_)) => head.push(byte[0]),
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => return Ok(head),
        }
    }

    if head != PROMPT_MARKER {
        return Ok(head);
    }

    let mut rest = [0; PROMPT_BYTES - 4];
    stream.read_exact(&mut rest).await?;

    let name = &rest[8..];
    let end = name.iter().position(|&byte| byte == 0).unwrap_or(name.len());
    Err(Error::OtherUser(String::from_utf8_lossy(&name[..end]).into_owned()))
}

// Finds the message with the key exchange rectangle. Stopping at that message leaves the following bytes as records.
fn key_exchange(messages: &mut Messages, early: &mut Vec<Rect>) -> Result<Option<[u8; 36]>> {
    while let Some(message) = messages.pop()? {
        let ServerMessage::FramebufferUpdate(rects) = message else {
            continue;
        };

        let mut exchange = None;

        for rect in rects {
            match rect {
                Rect::KeyExchange(body) => exchange = Some(body),
                rect => early.push(rect),
            }
        }

        if exchange.is_some() {
            return Ok(exchange);
        }
    }

    Ok(None)
}

// Authentication messages from the server have a `u32` length prefix.
async fn message(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let len = stream.read_u32().await? as usize;

    if len > MAX_MESSAGE_BYTES {
        return Err(Error::TooLong {
            what: "authentication message",
            len,
        });
    }

    let mut body = vec![0; len];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

async fn within<T, E: Into<Error>>(future: impl Future<Output = Result<T, E>>) -> Result<T> {
    timeout(STEP_TIMEOUT, future)
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::computer_name;

    #[test]
    fn computer_name_skips_the_binary_header() {
        let mut bytes = vec![0, 0, 0, 0, 0, 0x52, 0xa1, 0xb2, 0xc3, 0x2f, 0xd4];
        bytes.extend([0; 12]);
        bytes.extend("김민지’s Mac mini".as_bytes());

        assert_eq!(computer_name(&bytes), "김민지’s Mac mini");
        assert_eq!(computer_name(b"Mac mini"), "Mac mini");
        assert_eq!(computer_name(&[0x52, 0]), "");
    }
}
