//! Mac에 직접 접속해 각 단계를 확인하는 도구입니다.
//!
//! ```sh
//! # 인증 없이 보안 유형과 서버 공개키만 확인합니다.
//! cargo run --example probe -- mac-mini.local --handshake-only
//!
//! # 인증부터 첫 키프레임까지 확인하고, 받은 영상을 Annex-B 파일로 저장합니다.
//! NARU_PASSWORD=… cargo run --example probe -- mac-mini.local insd --out capture.h264 --seconds 10
//! ```
//!
//! - 비밀번호는 `NARU_PASSWORD`에서 읽고, 없으면 터미널에서 묻습니다. 비밀번호, 키, 클립보드 내용은 출력하지 않습니다.
//! - 서버 공개키는 `~/.naru-probe/<호스트>.der`에 저장하고, 다음 접속에서 바뀌었으면 접속을 멈춥니다.
//! - 저장한 파일은 `ffplay capture.h264`로 재생할 수 있습니다.
//! - `--unlock 3`은 3초 뒤 잠금 화면에 접속 비밀번호를 키 입력으로 칩니다. 가상 디스플레이로 붙으면 잠금 화면부터 나옵니다.
//! - `--resize 30:1600x1000@2`는 30초 뒤 가상 디스플레이의 크기를 바꿉니다. 서버가 새 SSRC로 넘어가는 장면을 캡처에 담을 때 씁니다.
use protocol::auth::{KEY_REQUEST, MAX_MESSAGE_BYTES, ServerKey};
use protocol::{Config, DecoderConfig, DisplayConfig, Event, MediaStats, Mode, Session};
use sha2::{Digest, Sha256};
use std::error::Error as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const USAGE: &str = "사용법: probe <호스트> [사용자] [--port N] [--local-ports A,B] [--display 1280x800@2] [--resize 30:1600x1000@2] [--unlock N] [--audio] [--out 파일] [--seconds N] [--handshake-only]";

#[tokio::main]
async fn main() {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            std::process::exit(2);
        }
    };

    let result = if args.handshake_only {
        handshake(&args).await
    } else {
        session(args).await
    };

    if let Err(message) = result {
        eprintln!("[실패] {message}");
        std::process::exit(1);
    }
}

struct Args {
    host: String,
    username: Option<String>,
    port: u16,
    local_ports: Option<(u16, u16)>,
    display: Option<Mode>,
    resize: Option<(u64, Mode)>,
    unlock: Option<u64>,
    audio: bool,
    out: Option<PathBuf>,
    seconds: u64,
    handshake_only: bool,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut parsed = Self {
            host: String::new(),
            username: None,
            port: 5900,
            local_ports: None,
            display: None,
            resize: None,
            unlock: None,
            audio: false,
            out: None,
            seconds: 10,
            handshake_only: false,
        };

        let mut positional = Vec::new();

        while let Some(arg) = args.next() {
            let mut value = || args.next().ok_or(format!("{arg}에 값이 필요합니다"));

            match arg.as_str() {
                "--port" => parsed.port = value()?.parse().map_err(|_| "포트가 올바르지 않습니다")?,
                "--local-ports" => parsed.local_ports = Some(pair(&value()?, ',').ok_or("--local-ports A,B")?),
                "--display" => parsed.display = Some(display(&value()?).ok_or("--display 1280x800@2")?),
                "--resize" => {
                    let text = value()?;
                    let parsed_resize = text
                        .split_once(':')
                        .and_then(|(at, mode)| Some((at.parse().ok()?, display(mode)?)));
                    parsed.resize = Some(parsed_resize.ok_or("--resize 30:1600x1000@2")?);
                }
                "--unlock" => parsed.unlock = Some(value()?.parse().map_err(|_| "--unlock N")?),
                "--audio" => parsed.audio = true,
                "--out" => parsed.out = Some(value()?.into()),
                "--seconds" => parsed.seconds = value()?.parse().map_err(|_| "--seconds N")?,
                "--handshake-only" => parsed.handshake_only = true,
                _ if arg.starts_with("--") => return Err(format!("알 수 없는 옵션: {arg}")),
                _ => positional.push(arg),
            }
        }

        let mut positional = positional.into_iter();
        parsed.host = positional.next().ok_or("호스트가 필요합니다")?;
        parsed.username = positional.next();

        if !parsed.handshake_only && parsed.username.is_none() {
            return Err("사용자 이름이 필요합니다".into());
        }

        Ok(parsed)
    }
}

// 인증 전까지만 진행해 보안 유형과 서버 공개키를 확인합니다.
async fn handshake(args: &Args) -> Result<(), String> {
    let started = Instant::now();
    let mut stream = TcpStream::connect((args.host.as_str(), args.port))
        .await
        .map_err(|error| error.to_string())?;
    println!(
        "[{:>6}] TCP 연결: {}",
        elapsed(started),
        stream.peer_addr().map_err(|error| error.to_string())?
    );

    let mut version = [0; 12];
    stream
        .read_exact(&mut version)
        .await
        .map_err(|error| error.to_string())?;
    println!(
        "[{:>6}] 서버 버전: {:?}",
        elapsed(started),
        String::from_utf8_lossy(&version).trim_end()
    );
    stream
        .write_all(b"RFB 003.889\n")
        .await
        .map_err(|error| error.to_string())?;

    let count = stream.read_u8().await.map_err(|error| error.to_string())?;
    let mut types = vec![0; usize::from(count)];
    stream.read_exact(&mut types).await.map_err(|error| error.to_string())?;
    println!("[{:>6}] 보안 유형: {types:?}", elapsed(started));

    stream
        .write_all(&KEY_REQUEST)
        .await
        .map_err(|error| error.to_string())?;
    let len = stream.read_u32().await.map_err(|error| error.to_string())? as usize;

    if len > MAX_MESSAGE_BYTES {
        return Err(format!("키 응답 길이 {len}이(가) 한도를 넘습니다"));
    }

    let mut body = vec![0; len];
    stream.read_exact(&mut body).await.map_err(|error| error.to_string())?;
    let key = ServerKey::parse(&body).map_err(|error| error.to_string())?;

    println!(
        "[{:>6}] 키 응답 {len}바이트, DER {}바이트",
        elapsed(started),
        key.der().len()
    );
    println!(
        "[{:>6}] 공개키 SHA-256: {}",
        elapsed(started),
        hex(&Sha256::digest(key.der()))
    );

    let Some(username) = &args.username else {
        return Ok(());
    };

    // 사용자 이름만 보내고 SRP 챌린지의 구조를 확인합니다. 비밀번호 증명은 보내지 않습니다.
    let identity = key.identity(username).map_err(|error| error.to_string())?;
    stream.write_all(&identity).await.map_err(|error| error.to_string())?;

    let len = stream.read_u32().await.map_err(|error| error.to_string())? as usize;

    if len > MAX_MESSAGE_BYTES {
        return Err(format!("챌린지 길이 {len}이(가) 한도를 넘습니다"));
    }

    let mut body = vec![0; len];
    stream.read_exact(&mut body).await.map_err(|error| error.to_string())?;
    println!("[{:>6}] 챌린지 {len}바이트", elapsed(started));
    println!("{}", layout(&body));
    Ok(())
}

// 챌린지의 머리 바이트와 길이 필드의 위치만 보여 줍니다. salt와 B 같은 값은 출력하지 않습니다.
fn layout(body: &[u8]) -> String {
    let head = &body[..body.len().min(16)];
    let mut lines = vec![format!("  머리 16바이트: {}", hex(head))];

    // 길이 접두로 보이는 위치를 찾기 위해, 앞부분의 u16 값과 오프셋을 나열합니다.
    for at in (0..body.len().min(24)).step_by(1) {
        if let Some(pair) = body.get(at..at + 2) {
            let value = u16::from_be_bytes([pair[0], pair[1]]);

            if (1..=1024).contains(&value) {
                lines.push(format!("  +{at:#04x} u16 = {value}"));
            }
        }
    }

    // 옵션 문자열은 공개된 이름이므로 위치와 함께 보여 줍니다.
    if let Some(at) = body.windows(4).position(|window| window == b"mda=") {
        let end = body[at..]
            .iter()
            .position(|&byte| byte == 0)
            .map_or(body.len(), |end| at + end);
        let options = String::from_utf8_lossy(&body[at..end.min(at + 120)]);
        lines.push(format!("  옵션 문자열 +{at:#x}: {options}"));
        lines.push(format!("  옵션 앞 10바이트: {}", hex(&body[at.saturating_sub(10)..at])));
    }

    lines.join("\n")
}

async fn session(args: Args) -> Result<(), String> {
    let username = args.username.clone().unwrap_or_default();

    let password = match std::env::var("NARU_PASSWORD") {
        Ok(password) => password,
        Err(_) => rpassword::prompt_password(format!("{username}@{}의 비밀번호: ", args.host))
            .map_err(|error| error.to_string())?,
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
    config.local_ports = args.local_ports;
    config.audio = args.audio;
    config.known_key = std::fs::read(&key_path).ok();

    config.display = args.display.map(display_config);

    let mut out = match &args.out {
        Some(path) => Some(BufWriter::new(File::create(path).map_err(|error| error.to_string())?)),
        None => None,
    };

    let (audio_port, video_port) = args.local_ports.unwrap_or((args.port, args.port + 1));
    println!("로컬 UDP 포트: 오디오 {audio_port}, 비디오 {video_port}");

    let started = Instant::now();
    let (session, mut events) = Session::start(config);
    let stop = tokio::time::sleep(Duration::from_secs(args.seconds));
    tokio::pin!(stop);

    let resize_at = args
        .resize
        .map_or(Duration::MAX, |(seconds, _)| Duration::from_secs(seconds));
    let resize = tokio::time::sleep(resize_at.min(Duration::from_secs(u32::MAX.into())));
    tokio::pin!(resize);
    let mut resized = args.resize.is_none();

    let unlock = tokio::time::sleep(Duration::from_secs(args.unlock.unwrap_or(u32::MAX.into())));
    tokio::pin!(unlock);
    let mut unlocked = args.unlock.is_none();

    let mut stats = Stats::default();
    let mut decoder: Option<DecoderConfig> = None;
    let mut closing = false;

    loop {
        let event = tokio::select! {
            event = events.recv() => event,
            _ = &mut stop, if !closing => {
                println!("[{:>6}] {}초가 지나 세션을 닫습니다", elapsed(started), args.seconds);
                closing = true;
                let _ = session.close();
                continue;
            }
            _ = &mut resize, if !resized => {
                resized = true;

                if let Some((_, mode)) = args.resize {
                    println!("[{:>6}] 해상도 변경 요청: 논리 {:?}, 픽셀 {:?}", elapsed(started), mode.logical, mode.pixels);

                    if let Err(error) = session.resize(&display_config(mode)) {
                        println!("[{:>6}] 해상도 변경 실패: {error}", elapsed(started));
                    }
                }

                continue;
            }
            _ = &mut unlock, if !unlocked => {
                unlocked = true;

                if let Some(text) = unlock_text.clone() {
                    println!("[{:>6}] 잠금 화면에 비밀번호 입력({}자)", elapsed(started), text.chars().count());
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
            Event::Progress(stage) => println!("[{at:>6}] 단계: {stage}"),
            Event::ServerKey(der) => {
                println!("[{at:>6}] 공개키 SHA-256: {}", hex(&Sha256::digest(&der)));

                if !key_path.exists() {
                    let _ = std::fs::create_dir_all(key_path.parent().unwrap_or(&key_path));
                    let _ = std::fs::write(&key_path, &der);
                    println!("[{at:>6}] 처음 본 키를 {}에 저장했습니다", key_path.display());
                }
            }
            Event::Authenticated { server_proof } => {
                println!("[{at:>6}] 인증 성공 (서버 증명 확인: {server_proof})");
            }
            Event::Layout(layout) => {
                println!(
                    "[{at:>6}] 레이아웃: 논리 {:?}, 픽셀 {:?}, 디스플레이 {}개",
                    layout.logical,
                    layout.pixels,
                    layout.displays.len()
                );
            }
            Event::DecoderConfig(config) => {
                println!(
                    "[{at:>6}] 디코더 설정: 프로파일 {}, 레벨 {}, SPS {}개, PPS {}개",
                    config.profile,
                    config.level,
                    config.sps.len(),
                    config.pps.len()
                );

                decoder = Some(config);
            }
            Event::Frame(unit) => {
                stats.frame(unit.keyframe, unit.nals.iter().map(Vec::len).sum());

                if unit.keyframe {
                    let types: Vec<u8> = unit
                        .nals
                        .iter()
                        .filter_map(|nal| nal.first().map(|header| header & 0x1f))
                        .collect();
                    println!("[{at:>6}] 키프레임: SSRC {:#x}, NAL 유형 {types:?}", unit.ssrc);
                }

                if let Some(out) = out.as_mut() {
                    // 디코더 설정은 스트림 밖으로 오므로, 키프레임 앞에 SPS와 PPS를 붙여야 파일만으로 재생됩니다.
                    if unit.keyframe
                        && !unit.has_sps()
                        && let Some(config) = &decoder
                    {
                        out.write_all(&config.annex_b()).map_err(|error| error.to_string())?;
                    }

                    out.write_all(&unit.annex_b()).map_err(|error| error.to_string())?;
                }

                if let Some(line) = stats.report(started) {
                    println!("[{at:>6}] {line}");
                }
            }
            Event::Cursor(image) => println!(
                "[{at:>6}] 커서: {}×{}, 핫스팟 {:?}",
                image.width, image.height, image.hotspot
            ),
            Event::InputSource(source) => println!("[{at:>6}] 입력 소스: {source}"),
            Event::Clipboard(text) => println!("[{at:>6}] 클립보드: {}자", text.chars().count()),
            Event::ResetDecoder => println!("[{at:>6}] 디코더 재생성 요청"),
            Event::Stats(media) => {
                if let Some(line) = stats.media(media) {
                    println!("[{at:>6}] {line}");
                }
            }
            Event::UnknownRect { encoding, len } => {
                println!("[{at:>6}] 모르는 사각형: 인코딩 {encoding:#x}, {len}바이트")
            }
            Event::Closed(None) => {
                println!("[{at:>6}] 세션 종료");
                break;
            }
            Event::Closed(Some(error)) => {
                let mut message = error.to_string();
                let mut source = error.source();

                while let Some(cause) = source {
                    message.push_str(&format!(": {cause}"));
                    source = cause.source();
                }

                return Err(message);
            }
            _ => {}
        }
    }

    if let Some(mut out) = out {
        out.flush().map_err(|error| error.to_string())?;
    }

    println!(
        "프레임 {}개(키프레임 {}개), {} KiB",
        stats.frames,
        stats.keyframes,
        stats.bytes >> 10
    );
    Ok(())
}

#[derive(Default)]
struct Stats {
    frames: u64,
    keyframes: u64,
    bytes: usize,
    reported: Option<(Instant, u64)>,
    last_frame: Option<Instant>,
    media_reported: Option<Instant>,
    offers_reported: bool,
}

impl Stats {
    fn frame(&mut self, keyframe: bool, bytes: usize) {
        self.frames += 1;
        self.keyframes += u64::from(keyframe);
        self.bytes += bytes;
        self.last_frame = Some(Instant::now());
    }

    // 프레임이 1초 넘게 멈추면 0.5초마다, 아니면 5초마다 수신 통계를 보고합니다.
    fn media(&mut self, media: MediaStats) -> Option<String> {
        let now = Instant::now();
        let stalled = self
            .last_frame
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1));
        let due = self
            .media_reported
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(5));

        if !stalled && !due {
            return None;
        }

        self.media_reported = Some(now);

        let offers = if self.offers_reported {
            String::new()
        } else {
            self.offers_reported = true;
            format!(", 보낸 오퍼 {}개", media.offers)
        };

        Some(format!(
            "수신{}: 데이터그램 {}, 인증 통과 {}, SSRC {}, 최고 순번 {}, 후보 SSRC {}개{offers}",
            if stalled { "(프레임 멈춤)" } else { "" },
            media.datagrams,
            media.authenticated,
            media.ssrc.map_or("없음".into(), |ssrc| format!("{ssrc:#x}")),
            media.highest.map_or("없음".into(), |highest| format!(
                "{}:{}",
                highest >> 16,
                highest & 0xffff
            )),
            media.candidates,
        ))
    }

    // 1초마다 초당 프레임 수를 보고합니다.
    fn report(&mut self, started: Instant) -> Option<String> {
        let now = Instant::now();
        let (since, frames) = *self.reported.get_or_insert((started, 0));
        let elapsed = now.duration_since(since);

        if elapsed < Duration::from_secs(1) {
            return None;
        }

        self.reported = Some((now, self.frames));
        let rate = (self.frames - frames) as f64 / elapsed.as_secs_f64();
        Some(format!("{rate:.1} fps, 누적 {}프레임", self.frames))
    }
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

fn display_config(mode: Mode) -> DisplayConfig {
    DisplayConfig {
        name: "Naru".into(),
        physical_mm: (369.45, 207.82),
        modes: vec![mode],
    }
}

fn pair(text: &str, separator: char) -> Option<(u16, u16)> {
    let (first, second) = text.split_once(separator)?;
    Some((first.parse().ok()?, second.parse().ok()?))
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
    format!("{:.2}s", started.elapsed().as_secs_f64())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
