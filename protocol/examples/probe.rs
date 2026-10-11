//! Mac에 직접 접속해 어느 단계까지 되는지 확인하고, 받은 영상을 파일로 남기는 진단 도구입니다.
//!
//! ```sh
//! # 인증 없이 버전, 보안 유형, 서버 공개키만 확인합니다.
//! cargo run --example probe -- mac-mini.local --handshake-only
//!
//! # 접속부터 미디어 수신까지 확인하고, 받은 영상을 Annex-B 파일로 저장합니다.
//! NARU_PASSWORD=… cargo run --example probe -- mac-mini.local insd --display 1280x800@2 --unlock 10 --out capture.h264 --seconds 30
//! ```
//!
//! - 비밀번호는 `NARU_PASSWORD`에서 읽고, 없으면 터미널에서 묻습니다. 비밀번호, 키, 클립보드 내용은 출력하지 않습니다.
//! - 서버 공개키는 `~/.naru-probe/<호스트>.der`에 저장하고, 다음 접속에서 바뀌었으면 접속을 멈춥니다.
//! - 가상 디스플레이(`--display`)로 붙으면 잠금 화면부터 나옵니다. `--unlock N`은 N초 뒤 접속 비밀번호를 키 입력으로 쳐서 풉니다.
//! - 저장한 파일은 `ffplay capture.h264`로 재생할 수 있습니다.
use anyhow::{Context, Result, bail};
use protocol::auth::{KEY_REQUEST, MAX_MESSAGE_BYTES, ServerKey, VERSION};
use protocol::{Config, DecoderConfig, DisplayConfig, Event, Mode, Session};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const USAGE: &str = "사용법: probe <호스트> [사용자] [--port N] [--display 1280x800@2] [--unlock N] [--audio] [--out 파일] [--seconds N] [--handshake-only]";

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse(std::env::args().skip(1)).context(USAGE)?;

    if args.handshake_only {
        handshake(&args).await
    } else {
        session(args).await
    }
}

struct Args {
    host: String,
    username: Option<String>,
    port: u16,
    display: Option<Mode>,
    unlock: Option<u64>,
    audio: bool,
    out: Option<PathBuf>,
    seconds: u64,
    handshake_only: bool,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut parsed = Self {
            host: String::new(),
            username: None,
            port: 5900,
            display: None,
            unlock: None,
            audio: false,
            out: None,
            seconds: 10,
            handshake_only: false,
        };

        let mut positional = Vec::new();

        while let Some(arg) = args.next() {
            let mut value = || args.next().with_context(|| format!("{arg}에 값이 필요합니다"));

            match arg.as_str() {
                "--port" => parsed.port = value()?.parse()?,
                "--display" => parsed.display = Some(display(&value()?).context("--display 1280x800@2")?),
                "--unlock" => parsed.unlock = Some(value()?.parse()?),
                "--audio" => parsed.audio = true,
                "--out" => parsed.out = Some(value()?.into()),
                "--seconds" => parsed.seconds = value()?.parse()?,
                "--handshake-only" => parsed.handshake_only = true,
                _ if arg.starts_with("--") => bail!("알 수 없는 옵션입니다: {arg}"),
                _ => positional.push(arg),
            }
        }

        let mut positional = positional.into_iter();
        parsed.host = positional.next().context("호스트가 필요합니다")?;
        parsed.username = positional.next();
        (parsed.handshake_only || parsed.username.is_some())
            .then_some(())
            .context("사용자 이름이 필요합니다")?;
        Ok(parsed)
    }
}

// 인증 전까지만 진행해 버전, 보안 유형, 서버 공개키를 확인합니다.
async fn handshake(args: &Args) -> Result<()> {
    let started = Instant::now();
    let mut stream = TcpStream::connect((args.host.as_str(), args.port)).await?;
    println!("[{}] TCP 연결: {}", elapsed(started), stream.peer_addr()?);

    let mut version = [0; 12];
    stream.read_exact(&mut version).await?;
    println!(
        "[{}] 서버 버전: {:?}",
        elapsed(started),
        String::from_utf8_lossy(&version).trim_end()
    );
    stream.write_all(VERSION).await?;

    let count = stream.read_u8().await?;
    let mut types = vec![0; usize::from(count)];
    stream.read_exact(&mut types).await?;
    println!("[{}] 보안 유형: {types:?}", elapsed(started));

    stream.write_all(&KEY_REQUEST).await?;
    let len = stream.read_u32().await? as usize;
    (len <= MAX_MESSAGE_BYTES)
        .then_some(())
        .context("키 응답이 한도보다 깁니다")?;

    let mut body = vec![0; len];
    stream.read_exact(&mut body).await?;
    let key = ServerKey::parse(&body)?;
    println!(
        "[{}] 공개키 SHA-256: {}",
        elapsed(started),
        hex(&Sha256::digest(key.der()))
    );
    Ok(())
}

async fn session(args: Args) -> Result<()> {
    let username = args.username.clone().unwrap_or_default();
    let password = match std::env::var("NARU_PASSWORD") {
        Ok(password) => password,
        Err(_) => rpassword::prompt_password(format!("{username}@{}의 비밀번호: ", args.host))?,
    };

    let key_path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".naru-probe")
        .join(format!("{}.der", args.host));

    // 잠금 화면에 칠 비밀번호입니다. 화면에 찍지 않고 키 입력으로만 보냅니다.
    let unlock_text = args.unlock.map(|_| password.clone());

    let mut config = Config::new(&args.host, username, password);
    config.port = args.port;
    config.audio = args.audio;
    config.known_key = std::fs::read(&key_path).ok();
    config.display = args.display.map(DisplayConfig::from);

    let mut out = args.out.as_ref().map(File::create).transpose()?.map(BufWriter::new);

    let started = Instant::now();
    let (session, mut events) = Session::start(config);

    let stop = tokio::time::sleep(Duration::from_secs(args.seconds));
    let unlock = tokio::time::sleep(Duration::from_secs(args.unlock.unwrap_or(args.seconds)));
    tokio::pin!(stop, unlock);

    let mut unlocked = args.unlock.is_none();
    let mut closing = false;
    let mut decoder: Option<DecoderConfig> = None;
    let (mut frames, mut keyframes, mut bytes) = (0u64, 0u64, 0usize);

    loop {
        let event = tokio::select! {
            event = events.recv() => event,
            _ = &mut stop, if !closing => {
                println!("[{}] {}초가 지나 세션을 닫습니다", elapsed(started), args.seconds);
                closing = true;
                let _ = session.close();
                continue;
            }
            _ = &mut unlock, if !unlocked => {
                unlocked = true;

                if let Some(text) = unlock_text.clone() {
                    println!("[{}] 잠금 화면에 비밀번호 입력({}자)", elapsed(started), text.chars().count());
                    tokio::spawn(type_text(session.clone(), text));
                }

                continue;
            }
            _ = tokio::signal::ctrl_c(), if !closing => {
                closing = true;
                let _ = session.close();
                continue;
            }
        };

        let Some(event) = event else {
            break;
        };

        let at = elapsed(started);

        match event {
            Event::Progress(stage) => println!("[{at}] 단계: {stage}"),
            Event::ServerKey(der) => {
                println!("[{at}] 공개키 SHA-256: {}", hex(&Sha256::digest(&der)));

                if !key_path.exists() {
                    std::fs::create_dir_all(key_path.parent().unwrap_or(&key_path))?;
                    std::fs::write(&key_path, &der)?;
                    println!("[{at}] 처음 본 키를 {}에 저장했습니다", key_path.display());
                }
            }
            Event::ServerName(name) => println!("[{at}] Mac 이름: {name}"),
            Event::Layout(layout) => println!("[{at}] 레이아웃: 논리 {:?}, 픽셀 {:?}", layout.logical, layout.pixels),
            Event::DecoderConfig(config) => {
                println!("[{at}] 디코더 설정: 프로파일 {}, 레벨 {}", config.profile, config.level);
                decoder = Some(config);
            }
            Event::Frame(unit) => {
                frames += 1;
                keyframes += u64::from(unit.keyframe);
                bytes += unit.nals.iter().map(Vec::len).sum::<usize>();

                if let Some(out) = out.as_mut() {
                    // 디코더 설정은 스트림 밖으로 오므로, 키프레임 앞에 SPS와 PPS를 붙여야 파일만으로 재생됩니다.
                    if unit.keyframe
                        && !unit.has_sps()
                        && let Some(config) = &decoder
                    {
                        out.write_all(&config.annex_b())?;
                    }

                    out.write_all(&unit.annex_b())?;
                }
            }
            Event::InputSource(source) => println!("[{at}] 입력 소스: {} (보안 입력 {})", source.id, source.secure),
            Event::Closed(None) => break,
            Event::Closed(Some(error)) => return Err(error.into()),
            _ => {}
        }
    }

    if let Some(mut out) = out {
        out.flush()?;
    }

    println!("프레임 {frames}개(키프레임 {keyframes}개), {} KiB", bytes >> 10);
    Ok(())
}

// 글자마다 키를 눌렀다 떼고 마지막에 Return을 누릅니다. 라틴-1 밖의 글자는 유니코드 keysym(0x0100_0000 + 코드 포인트)으로 보냅니다.
async fn type_text(session: Session, text: String) {
    const RETURN: u32 = 0xff0d;
    const PAUSE: Duration = Duration::from_millis(40);

    let keysyms = text
        .chars()
        .map(|char| match u32::from(char) {
            code @ (0x20..=0x7e | 0xa0..=0xff) => code,
            code => 0x0100_0000 + code,
        })
        .chain([RETURN]);

    for keysym in keysyms {
        for down in [true, false] {
            if session.key(keysym, down).is_err() {
                return;
            }

            tokio::time::sleep(PAUSE).await;
        }
    }
}

// `1280x800@2` → 논리 1280×800, 배율 2
fn display(text: &str) -> Option<Mode> {
    let (size, scale) = text.split_once('@').unwrap_or((text, "1"));
    let (width, height) = size.split_once('x')?;
    Some(Mode::new(
        (width.parse().ok()?, height.parse().ok()?),
        scale.parse().ok()?,
    ))
}

fn elapsed(started: Instant) -> String {
    format!("{:>6.2}s", started.elapsed().as_secs_f64())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
