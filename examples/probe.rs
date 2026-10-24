//! A diagnostic tool that connects to a Mac directly, shows how far the connection gets, and saves the received video to a file.
//!
//! ```sh
//! # Checks only the version, security types, and server public key, without authenticating.
//! cargo run --example probe -- mac-mini.local --handshake-only
//!
//! # Goes from connecting through media reception and saves the received video as an Annex-B file.
//! NARU_PASSWORD=… cargo run --example probe -- mac-mini.local insd --display 1280x800@2 --unlock 10 --out capture.h264 --seconds 30
//! ```
//!
//! - The password is read from `NARU_PASSWORD`, or prompted in the terminal. Passwords, keys, and clipboard contents are never printed.
//! - The server public key is saved to `~/.naru-probe/<host>.der`, and a later connection stops if it changed.
//! - With a virtual display (`--display`) the session starts at the lock screen. `--unlock N` types the login password after N seconds to unlock it.
//! - Play the saved file with `ffplay capture.h264`.
//! - `--hevc` requests HEVC 4:4:4 instead of H.264, and `--out` then saves HEVC Annex-B. The four bands go into the file
//!   in decoding order, so `ffmpeg -i capture.hevc -f null -` checks that they decode, but a player shows them one by one.
//! - `--local-ports audio,video` changes the local UDP ports, for when Apple's Screen Sharing holds 5900 and 5901 on the same Mac.
use anyhow::{Context, Result, bail};
use naru::auth::{KEY_REQUEST, MAX_MESSAGE_BYTES, ServerKey, VERSION};
use naru::{Config, DecoderConfig, DisplayConfig, Event, Mode, Session, VideoCodec};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const USAGE: &str = "usage: probe <host> [user] [--port N] [--local-ports N,N] [--display 1280x800@2] [--unlock N] [--audio] [--out FILE] [--hevc] [--seconds N] [--handshake-only]";

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
    local_ports: Option<(u16, u16)>,
    display: Option<Mode>,
    unlock: Option<u64>,
    audio: bool,
    out: Option<PathBuf>,
    hevc: bool,
    seconds: u64,
    handshake_only: bool,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut parsed = Self {
            host: String::new(),
            username: None,
            port: 5900,
            local_ports: None,
            display: None,
            unlock: None,
            audio: false,
            out: None,
            hevc: false,
            seconds: 10,
            handshake_only: false,
        };

        let mut positional = Vec::new();

        while let Some(arg) = args.next() {
            let mut value = || args.next().with_context(|| format!("{arg} needs a value"));

            match arg.as_str() {
                "--port" => parsed.port = value()?.parse()?,
                "--local-ports" => {
                    let text = value()?;
                    let (audio, video) = text.split_once(',').context("--local-ports 15900,15901")?;
                    parsed.local_ports = Some((audio.parse()?, video.parse()?));
                }
                "--display" => parsed.display = Some(display(&value()?).context("--display 1280x800@2")?),
                "--unlock" => parsed.unlock = Some(value()?.parse()?),
                "--audio" => parsed.audio = true,
                "--out" => parsed.out = Some(value()?.into()),
                "--hevc" => parsed.hevc = true,
                "--seconds" => parsed.seconds = value()?.parse()?,
                "--handshake-only" => parsed.handshake_only = true,
                _ if arg.starts_with("--") => bail!("unknown option: {arg}"),
                _ => positional.push(arg),
            }
        }

        let mut positional = positional.into_iter();
        parsed.host = positional.next().context("a host is required")?;
        parsed.username = positional.next();
        (parsed.handshake_only || parsed.username.is_some())
            .then_some(())
            .context("a username is required")?;
        Ok(parsed)
    }
}

// Goes only as far as authentication to check the version, security types, and server public key.
async fn handshake(args: &Args) -> Result<()> {
    let started = Instant::now();
    let mut stream = TcpStream::connect((args.host.as_str(), args.port)).await?;
    println!("[{}] TCP connected: {}", elapsed(started), stream.peer_addr()?);

    let mut version = [0; 12];
    stream.read_exact(&mut version).await?;
    println!(
        "[{}] server version: {:?}",
        elapsed(started),
        String::from_utf8_lossy(&version).trim_end()
    );
    stream.write_all(VERSION).await?;

    let count = stream.read_u8().await?;
    let mut types = vec![0; usize::from(count)];
    stream.read_exact(&mut types).await?;
    println!("[{}] security types: {types:?}", elapsed(started));

    stream.write_all(&KEY_REQUEST).await?;
    let len = stream.read_u32().await? as usize;
    (len <= MAX_MESSAGE_BYTES)
        .then_some(())
        .context("the key response exceeds the limit")?;

    let mut body = vec![0; len];
    stream.read_exact(&mut body).await?;
    let key = ServerKey::parse(&body)?;
    println!(
        "[{}] public key SHA-256: {}",
        elapsed(started),
        hex(&Sha256::digest(key.der()))
    );
    Ok(())
}

async fn session(args: Args) -> Result<()> {
    let username = args.username.clone().unwrap_or_default();
    let password = match std::env::var("NARU_PASSWORD") {
        Ok(password) => password,
        Err(_) => rpassword::prompt_password(format!("password for {username}@{}: ", args.host))?,
    };

    let key_path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".naru-probe")
        .join(format!("{}.der", args.host));

    // The password to type at the lock screen. It is only sent as key input, never printed.
    let unlock_text = args.unlock.map(|_| password.clone());

    let mut config = Config::new(&args.host, username, password);
    config.port = args.port;
    config.local_ports = args.local_ports;
    config.audio = args.audio;
    config.known_key = std::fs::read(&key_path).ok();
    config.display = args.display.map(DisplayConfig::from);
    config.codec = if args.hevc { VideoCodec::Hevc } else { VideoCodec::H264 };

    let mut out = args.out.as_ref().map(File::create).transpose()?.map(BufWriter::new);

    let mut bands = vec![0u64; usize::from(config.codec.bands())];
    let started = Instant::now();
    let (session, mut events) = Session::start(config);

    let stop = tokio::time::sleep(Duration::from_secs(args.seconds));
    let unlock = tokio::time::sleep(Duration::from_secs(args.unlock.unwrap_or(args.seconds)));
    tokio::pin!(stop, unlock);

    let mut unlocked = args.unlock.is_none();
    let mut closing = false;
    let mut decoder: Option<DecoderConfig> = None;
    let (mut frames, mut keyframes, mut bytes) = (0u64, 0u64, 0usize);
    let mut last_frame = None;
    let mut largest_bytes = 0;

    loop {
        let event = tokio::select! {
            event = events.recv() => event,
            _ = &mut stop, if !closing => {
                println!("[{}] closing the session after {} seconds", elapsed(started), args.seconds);
                closing = true;
                session.close();
                continue;
            }
            _ = &mut unlock, if !unlocked => {
                unlocked = true;

                if let Some(text) = unlock_text.clone() {
                    println!("[{}] typing the password at the lock screen ({} characters)", elapsed(started), text.chars().count());
                    tokio::spawn(type_text(session.clone(), text));
                }

                continue;
            }
            _ = tokio::signal::ctrl_c(), if !closing => {
                closing = true;
                session.close();
                continue;
            }
        };

        let Some(event) = event else {
            break;
        };

        let at = elapsed(started);

        match event {
            Event::Progress(stage) => println!("[{at}] stage: {stage}"),
            Event::ServerKey(der) => {
                println!("[{at}] public key SHA-256: {}", hex(&Sha256::digest(&der)));

                if !key_path.exists() {
                    std::fs::create_dir_all(key_path.parent().unwrap_or(&key_path))?;
                    std::fs::write(&key_path, &der)?;
                    println!("[{at}] saved the new key to {}", key_path.display());
                }
            }
            Event::ServerName(name) => println!("[{at}] Mac name: {name}"),
            Event::Layout(layout) => println!(
                "[{at}] layout: logical {:?}, pixels {:?}",
                layout.logical, layout.pixels
            ),
            Event::DecoderConfig(config) => {
                println!(
                    "[{at}] decoder config: {:?}, {}x{}",
                    config.codec, config.width, config.height
                );
                decoder = Some(config);
            }
            Event::Frame(unit) => {
                frames += 1;
                keyframes += u64::from(unit.keyframe);
                bands[usize::from(unit.band)] += 1;
                last_frame = Some(at.clone());
                bytes += unit.nals.iter().map(Vec::len).sum::<usize>();

                if let Some(out) = out.as_mut() {
                    // The parameter sets come only in the decoder config, so they go before keyframes for the file to
                    // play on its own.
                    if unit.keyframe
                        && let Some(config) = &decoder
                    {
                        out.write_all(&config.annex_b())?;
                    }

                    out.write_all(&unit.annex_b())?;
                }
            }
            Event::Stats(stats) => largest_bytes = stats.largest_bytes,
            Event::InputSource(source) => {
                println!("[{at}] input source: {} (secure input {})", source.id, source.secure)
            }
            Event::Closed(None) => break,
            Event::Closed(Some(error)) => return Err(error.into()),
            _ => {}
        }
    }

    if let Some(mut out) = out {
        out.flush()?;
    }

    println!(
        "{frames} frames ({keyframes} keyframes, per band {bands:?}, last at {}), {} KiB, largest video datagram {largest_bytes} bytes",
        last_frame.as_deref().unwrap_or("-").trim(),
        bytes >> 10
    );
    Ok(())
}

// Presses and releases a key per character, then Return. Characters outside Latin-1 go as Unicode keysyms (0x0100_0000
// + code point).
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
            if session.key(keysym, None, down).is_err() {
                return;
            }

            tokio::time::sleep(PAUSE).await;
        }
    }
}

// `1280x800@2` → logical 1280×800, scale 2
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
