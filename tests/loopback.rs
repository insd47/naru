//! 루프백 주소의 가짜 서버로 접속부터 첫 키프레임 수신까지를 확인합니다.
//!
//! - 서버 쪽 SRP는 법이 소수이기만 하면 성립하므로 작은 소수(2^127 − 1)로 계산합니다. 클라이언트는 그룹의 크기를 가정하지 않습니다.
//! - 키 교체 메시지 바로 뒤에 레코드를 이어 붙여 보내, 평문과 레코드의 경계를 정확히 나누는지 확인합니다.
use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockEncrypt, KeyInit};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use naru::record::{Opener, RecordKeys, Sealer};
use naru::{Config, Event, RtpPacket, Session, SrtpContext};
use num_bigint_dig::BigUint;
use rand::rngs::OsRng;
use rsa::pkcs8::EncodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey};
use sha2::{Digest, Sha256, Sha512};
use std::io::Write;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::time::timeout;

const USERNAME: &str = "insd";
const PASSWORD: &str = "비밀번호";
const SALT: [u8; 32] = [3; 32];
const ITERATIONS: u64 = 10;
const OPTIONS: &[u8] = b"mda=SHA-512,replay_detection,conf+int=ChaCha20-Poly1305,kdf=SALTED-SHA512-PBKDF2";
const SERVER_SSRC: u32 = 0x5000;
const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x21];

#[tokio::test(flavor = "multi_thread")]
async fn session_reaches_the_first_keyframe_against_a_loopback_server() {
    let (listener, audio, video) = server_sockets().await;
    let port = listener.local_addr().unwrap().port();
    let local = (free_udp_port().await, free_udp_port().await);

    let server = tokio::spawn(serve(listener, audio, video, local.1));

    let mut config = Config::new("127.0.0.1", USERNAME, PASSWORD);
    config.port = port;
    config.local_ports = Some(local);

    let (session, mut events) = Session::start(config);
    let mut seen = Vec::new();

    let frame = timeout(Duration::from_secs(20), async {
        loop {
            match events.recv().await.expect("세션이 이벤트를 보낸다") {
                Event::Frame(unit) => break unit,
                Event::Closed(error) => panic!("세션이 끝났습니다: {error:?} / {seen:?}"),
                event => seen.push(format!("{event:?}")),
            }
        }
    })
    .await
    .expect("키프레임이 온다");

    assert!(frame.keyframe);
    assert_eq!(frame.nals, [IDR]);

    let stages: Vec<&String> = seen.iter().filter(|event| event.starts_with("Progress")).collect();
    assert_eq!(stages.len(), 6, "{stages:?}");
    assert!(seen.iter().any(|event| event.starts_with("Layout")));
    assert!(seen.iter().any(|event| event.starts_with("DecoderConfig")));
    assert!(seen.iter().any(|event| event == "Authenticated { server_proof: true }"));

    session.close();

    loop {
        if let Event::Closed(error) = events.recv().await.unwrap() {
            assert!(error.is_none(), "{error:?}");
            break;
        }
    }

    server.abort();
}

async fn server_sockets() -> (TcpListener, UdpSocket, UdpSocket) {
    loop {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let (Ok(audio), Ok(video)) = (
            UdpSocket::bind(("127.0.0.1", port)).await,
            UdpSocket::bind(("127.0.0.1", port + 1)).await,
        ) else {
            continue;
        };

        return (listener, audio, video);
    }
}

async fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn serve(listener: TcpListener, _audio: UdpSocket, video: UdpSocket, client_video_port: u16) {
    let (mut warmup, _) = listener.accept().await.unwrap();
    banner(&mut warmup).await;
    let _ = warmup.read(&mut [0; 16]).await;

    let (mut stream, _) = listener.accept().await.unwrap();
    banner(&mut stream).await;

    let wrap_key = authenticate(&mut stream).await;

    let mut client_init = [0; 1];
    stream.read_exact(&mut client_init).await.unwrap();
    assert_eq!(client_init, [0xc1]);

    let mut server_init = vec![0; 20];
    server_init.extend_from_slice(&4u32.to_be_bytes());
    server_init.extend_from_slice(b"mini");
    stream.write_all(&server_init).await.unwrap();

    // ViewerInfo(66) + 암호화 시작(12), 그리고 인코딩 목록(4 + 13 × 4)
    let mut prelude = vec![0; 66 + 12 + 56];
    stream.read_exact(&mut prelude).await.unwrap();
    assert_eq!(&prelude[66..70], [0x12, 0, 0, 1]);
    assert_eq!(&prelude[78..82], [2, 0, 0, 13]);

    let keys = RecordKeys {
        content_key: [0x11; 16],
        iv: [0x22; 16],
    };

    let mut sealer = Sealer::new(&keys);
    let mut opener = Opener::new(&keys);

    // 키 교체 사각형 바로 뒤에 레코드(상태 알림)를 붙여 한 번에 보냅니다.
    let mut exchange = vec![0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x04, 0x4f, 0, 0, 0, 1];
    exchange.extend_from_slice(&ecb(&wrap_key, keys.content_key));
    exchange.extend_from_slice(&ecb(&wrap_key, keys.iv));
    exchange.extend(sealer.seal(&[0x14, 0, 0, 4, 0, 1, 0, 12]));
    stream.write_all(&exchange).await.unwrap();

    let mut activate = [0; 8];
    stream.read_exact(&mut activate).await.unwrap();
    assert_eq!(activate, [0x12, 0, 0, 2, 0, 1, 0, 0]);

    let offer = loop {
        let body = record(&mut stream, &mut opener).await;

        if body.first() == Some(&0x1c) {
            break body;
        }
    };

    let audio_len = u16::from_be_bytes([offer[0x0a], offer[0x0b]]) as usize;
    let video_keys = 0x80 + audio_len;
    let video_receive: [u8; 46] = offer[video_keys + 46..video_keys + 92].try_into().unwrap();

    let mut update = vec![0, 0, 0, 2];
    update.extend(rect(0x451, &layout()));
    update.extend(rect(0x3f2, &answer(1280, 800)));
    stream.write_all(&sealer.seal(&update)).await.unwrap();

    let mut srtp = SrtpContext::new(&video_receive);
    let client: SocketAddr = ([127, 0, 0, 1], client_video_port).into();

    let packets = [(1, 0, false, config_packet()), (2, 0, true, IDR.to_vec())];

    for (sequence, timestamp, marker, payload) in packets {
        let packet = RtpPacket {
            marker,
            payload_type: 100,
            sequence,
            timestamp,
            ssrc: SERVER_SSRC,
            payload,
        };

        let protected = srtp.protect_rtp(&packet.encode()).unwrap();
        video.send_to(&protected, client).await.unwrap();
    }

    // 클라이언트는 같은 SSRC의 패킷이 다섯 개 모여야 그 스트림을 고르므로 P 프레임을 이어 보냅니다.
    for sequence in 3..8 {
        let packet = RtpPacket {
            marker: true,
            payload_type: 100,
            sequence,
            timestamp: u32::from(sequence) * 3000,
            ssrc: SERVER_SSRC,
            payload: vec![0x41, 0x9a],
        };

        video
            .send_to(&srtp.protect_rtp(&packet.encode()).unwrap(), client)
            .await
            .unwrap();
    }

    // 클라이언트가 닫을 때까지 레코드를 읽어 둡니다.
    loop {
        if stream.read(&mut [0; 1024]).await.unwrap_or(0) == 0 {
            break;
        }
    }
}

async fn banner(stream: &mut TcpStream) {
    stream.write_all(b"RFB 003.889\n").await.unwrap();
    let mut version = [0; 12];
    stream.read_exact(&mut version).await.unwrap();
    assert_eq!(&version, b"RFB 003.889\n");
    stream.write_all(&[4, 30, 33, 36, 35]).await.unwrap();
}

// 서버 쪽 SRP를 계산하고 래핑 키를 반환합니다.
async fn authenticate(stream: &mut TcpStream) -> [u8; 16] {
    let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let der = private.to_public_key().to_public_key_der().unwrap();

    let mut request = [0; 15];
    stream.read_exact(&mut request).await.unwrap();
    assert_eq!(&request[7..11], b"RSA1");

    let mut body = vec![0, 1];
    body.extend_from_slice(&(der.as_bytes().len() as u32).to_be_bytes());
    body.extend_from_slice(der.as_bytes());
    body.push(0);
    send_prefixed(stream, &body).await;

    let mut identity = vec![0; 654];
    stream.read_exact(&mut identity).await.unwrap();
    let plain = private.decrypt(Pkcs1v15Encrypt, &identity[14..270]).unwrap();
    assert_eq!(&plain[8..8 + USERNAME.len()], USERNAME.as_bytes());

    let n = (BigUint::from(1u8) << 127usize) - BigUint::from(1u8);
    let g = BigUint::from(5u8);
    let b = BigUint::from(0x0123_4567_89ab_cdefu64);

    let mut dk = [0; 128];
    pbkdf2::pbkdf2_hmac::<Sha512>(PASSWORD.as_bytes(), &SALT, ITERATIONS as u32, &mut dk);
    let x = number(&[&SALT, &hash(&[b":", &dk])]);
    let v = g.modpow(&x, &n);
    let k = number(&[&pad(&n), &pad(&g)]);
    let b_pub = (&k * &v + g.modpow(&b, &n)) % &n;

    let mut challenge = vec![0; 11];
    challenge.extend_from_slice(&512u16.to_be_bytes());
    challenge.extend_from_slice(&pad(&n));
    challenge.extend_from_slice(&[0, 1, 5, 32]);
    challenge.extend_from_slice(&SALT);
    challenge.extend_from_slice(&512u16.to_be_bytes());
    challenge.extend_from_slice(&pad(&b_pub));
    challenge.extend_from_slice(&ITERATIONS.to_be_bytes());
    challenge.extend_from_slice(&(OPTIONS.len() as u16).to_be_bytes());
    challenge.extend_from_slice(OPTIONS);

    let len = challenge.len();
    challenge[..4].copy_from_slice(&[0, 0, 0, 2]);
    challenge[4..6].copy_from_slice(&((len - 6) as u16).to_be_bytes());
    challenge[6..10].copy_from_slice(&((len - 10) as u32).to_be_bytes());
    send_prefixed(stream, &challenge).await;

    let mut proof = vec![0; 1080];
    stream.read_exact(&mut proof).await.unwrap();
    let a = BigUint::from_bytes_be(&proof[20..532]);
    let m1 = &proof[533..597];

    let u = number(&[&pad(&a), &pad(&b_pub)]);
    let s = (&a * v.modpow(&u, &n)).modpow(&b, &n);
    let key = hash(&[&pad(&s)]);

    let mut block = vec![0; 9];
    block.push(64);
    block.extend_from_slice(&hash(&[&pad(&a), m1, &key]));
    send_prefixed(stream, &block).await;
    stream.write_all(&0u32.to_be_bytes()).await.unwrap();

    Sha256::digest(key)[..16].try_into().unwrap()
}

async fn send_prefixed(stream: &mut TcpStream, body: &[u8]) {
    stream.write_all(&(body.len() as u32).to_be_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
}

async fn record(stream: &mut TcpStream, opener: &mut Opener) -> Vec<u8> {
    loop {
        if let Some(body) = opener.pop().unwrap() {
            return body;
        }

        let mut buffer = [0; 4096];
        let read = stream.read(&mut buffer).await.unwrap();
        assert!(read > 0, "클라이언트가 연결을 닫았습니다");
        opener.push(&buffer[..read]);
    }
}

fn rect(encoding: i32, body: &[u8]) -> Vec<u8> {
    let mut rect = vec![0; 8];
    rect.extend_from_slice(&encoding.to_be_bytes());
    rect.extend_from_slice(&(body.len() as u16).to_be_bytes());
    rect.extend_from_slice(body);
    rect
}

fn layout() -> Vec<u8> {
    let mut body = Vec::new();

    for value in [5u16, 1280, 800, 1280, 800] {
        body.extend_from_slice(&value.to_be_bytes());
    }

    body.extend_from_slice(&[0xff; 4]);
    body.extend_from_slice(&[0; 4]);
    body.extend_from_slice(&0u16.to_be_bytes());
    body
}

// 비디오 응답 protobuf(필드 5 안에 폭 4, 높이 5, 타일 6)를 zlib으로 압축해 plist에 담습니다.
fn answer(width: u16, height: u16) -> Vec<u8> {
    let varint = |value: u16| -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut value = value;

        while value >= 0x80 {
            bytes.push((value as u8 & 0x7f) | 0x80);
            value >>= 7;
        }

        bytes.push(value as u8);
        bytes
    };

    let mut video = vec![0x20];
    video.extend(varint(width));
    video.push(0x28);
    video.extend(varint(height));
    video.extend_from_slice(&[0x30, 1]);

    let mut blob = vec![0x2a, video.len() as u8];
    blob.extend(video);

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&blob).unwrap();

    let mut dict = plist::Dictionary::new();
    dict.insert("avcMediaStreamNegotiatorMode".into(), 7.into());
    dict.insert(
        "avcMediaStreamNegotiatorMediaBlob".into(),
        plist::Value::Data(encoder.finish().unwrap()),
    );

    let mut body = Vec::new();
    plist::Value::Dictionary(dict).to_writer_binary(&mut body).unwrap();
    body
}

fn config_packet() -> Vec<u8> {
    let record = [1, 0x64, 0, 0x34, 0xff, 0xe1, 0, 2, 0x67, 0x64, 1, 0, 1, 0x68];
    let mut packet = vec![0x92, 0, 0, b'a', b'v', b'c', b'1'];
    packet.extend_from_slice(&((record.len() + 8) as u32).to_be_bytes());
    packet.extend_from_slice(b"avcC");
    packet.extend_from_slice(&record);
    packet
}

fn ecb(key: &[u8; 16], block: [u8; 16]) -> [u8; 16] {
    let mut block = GenericArray::from(block);
    Aes128::new(key.into()).encrypt_block(&mut block);
    block.into()
}

fn pad(value: &BigUint) -> Vec<u8> {
    let bytes = value.to_bytes_be();
    let mut padded = vec![0; 512 - bytes.len()];
    padded.extend_from_slice(&bytes);
    padded
}

fn hash(parts: &[&[u8]]) -> [u8; 64] {
    let mut digest = Sha512::new();

    for part in parts {
        digest.update(part);
    }

    digest.finalize().into()
}

fn number(parts: &[&[u8]]) -> BigUint {
    BigUint::from_bytes_be(&hash(parts))
}
