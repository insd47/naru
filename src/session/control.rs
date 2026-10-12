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

// 예열 연결을 닫은 뒤 기다리는 시간입니다. 예열해 두면 잠금 화면에서 데스크톱으로 넘어갈 때 서버가 TCP를 끊지 않습니다.
const WARMUP_DELAY: Duration = Duration::from_millis(1400);

// 서버는 ViewerInfo와 암호화 시작 요청을 받은 뒤, 활성화 요청을 받은 뒤 잠시 시간이 필요합니다.
const PRELUDE_DELAY: Duration = Duration::from_millis(100);
const ACTIVATE_DELAY: Duration = Duration::from_millis(200);

// 다른 사용자가 로그인해 있으면 ServerInit 뒤에 세션 선택 프롬프트가 곧바로 옵니다.
const PROMPT_WAIT: Duration = Duration::from_millis(300);
const PROMPT_MARKER: [u8; 4] = [0, 0x4a, 0, 1];
const PROMPT_BYTES: usize = 76;

const STEP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_NAME_BYTES: usize = 1 << 10;
const READ_BYTES: usize = 64 << 10;

/// 레코드 계층까지 활성화된 제어 채널입니다.
pub struct Control {
    stream: TcpStream,
    keys: RecordKeys,
    sealer: Sealer,
    opener: Opener,
    // 평문 구간에서 키 교체와 함께 온 사각형. 커서나 레이아웃일 수 있습니다.
    early: Vec<Rect>,
}

impl Control {
    /// 예열 연결, 핸드셰이크, 인증, 평문 프렐류드를 거쳐 레코드 계층을 활성화합니다.
    ///
    /// - TCP는 UDP와 같은 IPv4 주소로 연결합니다. 호스트 이름으로 연결하면 TCP만 IPv6로 붙어 미디어가 오지 않을 수 있습니다.
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

    /// 레코드 계층의 키. 포인터 이벤트의 암호화에도 씁니다.
    pub fn keys(&self) -> &RecordKeys {
        &self.keys
    }

    /// 접속 과정에서 이미 받은 것: 평문 구간의 사각형과, 키 교체 직후 같은 읽기에 붙어 온 레코드의 본문
    pub fn backlog(&mut self) -> Result<(Vec<Rect>, Vec<Vec<u8>>)> {
        Ok((std::mem::take(&mut self.early), self.opened()?))
    }

    /// 다음으로 읽은 바이트에서 완성된 레코드의 본문을 꺼냅니다. 서버가 연결을 닫았으면 `Closed`입니다.
    pub async fn receive(&mut self, buffer: &mut [u8]) -> Result<Vec<Vec<u8>>> {
        let read = self.stream.read(buffer).await?;

        if read == 0 {
            return Err(Error::Closed);
        }

        self.opener.push(&buffer[..read]);
        self.opened()
    }

    /// 닫을 때 연결을 곧바로 끊게 합니다(RST). 서버 쪽 세션이 남지 않아야 다시 접속할 수 있습니다.
    pub fn abort(&self) {
        let _ = SockRef::from(&self.stream).set_linger(Some(Duration::ZERO));
    }

    fn opened(&mut self) -> Result<Vec<Vec<u8>>> {
        std::iter::from_fn(|| self.opener.pop().transpose()).collect()
    }

    /// 메시지를 레코드로 감싸 보냅니다.
    pub async fn send(&mut self, message: &[u8]) -> Result<()> {
        let records = self.sealer.seal(message);
        self.stream.write_all(&records).await?;
        Ok(())
    }

    /// 메시지들을 차례로 보냅니다.
    pub async fn send_all(&mut self, messages: Vec<Vec<u8>>) -> Result<()> {
        for message in messages {
            self.send(&message).await?;
        }

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
        return Err(Error::Malformed("RFB 버전"));
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

// 래핑 키와, 서버 증명(M2)을 결과 블록에서 찾았는지를 반환합니다.
async fn authenticate(stream: &mut TcpStream, key: &ServerKey, config: &Config) -> Result<([u8; 16], bool)> {
    stream.write_all(&key.identity(&config.username)?).await?;

    let challenge = Challenge::parse(&message(stream).await?)?;

    // PBKDF2 반복은 수백 밀리초가 걸리므로 런타임의 다른 작업을 막지 않게 합니다.
    let password = config.password.clone();
    let proof = tokio::task::spawn_blocking(move || challenge.respond(&password))
        .await
        .map_err(|_| Error::Closed)?;

    stream.write_all(&proof.packet).await?;

    // 증명 블록 뒤에 결과가 옵니다. 서버 증명의 위치가 확정되지 않았으므로 검증 결과로 접속을 막지는 않습니다.
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

    // 키 교체 사각형이 든 메시지가 끝나는 지점부터는 레코드입니다. 같은 읽기에 레코드가 이어 붙어 올 수 있습니다.
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

// ServerInit을 읽어 Mac의 컴퓨터 이름을 반환합니다.
async fn server_init(stream: &mut TcpStream) -> Result<String> {
    let mut fixed = [0; 24];
    stream.read_exact(&mut fixed).await?;

    let len = u32::from_be_bytes(fixed[20..24].try_into().expect("4바이트다")) as usize;

    if len > MAX_NAME_BYTES {
        return Err(Error::TooLong {
            what: "서버 이름", len
        });
    }

    let mut name = vec![0; len];
    stream.read_exact(&mut name).await?;
    Ok(computer_name(&name))
}

// 이름 칸에는 NUL을 섞은 이진 머리와 NUL 채움이 컴퓨터 이름 앞에 붙어 옵니다(2026-10-12 macOS 27 실측). 마지막 NUL 뒤가 이름입니다.
fn computer_name(bytes: &[u8]) -> String {
    let start = bytes.iter().rposition(|&byte| byte == 0).map_or(0, |at| at + 1);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

// ServerInit 뒤의 세션 선택 프롬프트를 확인합니다. 프롬프트가 아니면 읽은 바이트를 돌려줍니다.
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

// 키 교체 사각형이 든 메시지를 찾습니다. 그 메시지에서 멈춰야 뒤의 바이트가 레코드로 남습니다.
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

// 서버가 보내는 인증 메시지는 `u32` 길이 접두가 붙습니다.
async fn message(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let len = stream.read_u32().await? as usize;

    if len > MAX_MESSAGE_BYTES {
        return Err(Error::TooLong {
            what: "인증 메시지",
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
        bytes.extend("황인성’s Mac mini".as_bytes());

        assert_eq!(computer_name(&bytes), "황인성’s Mac mini");
        assert_eq!(computer_name(b"Mac mini"), "Mac mini");
        assert_eq!(computer_name(&[0x52, 0]), "");
    }
}
