use super::{Event, MediaStats};
use crate::error::Result;
use crate::media::{
    Assembled, Assembler, AudioFrame, Priority, Routed, RtpPacket, SrtpContext, Streams, VideoCodec, rtcp,
};
use crate::offer::Offer;
use socket2::{Domain, Socket, Type};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

const RECEIVE_BUFFER_BYTES: usize = 4 << 20;
const PUNCH_INTERVAL: Duration = Duration::from_millis(100);

// 오디오 keepalive는 페이로드 유형 101, 20ms(480 샘플)마다 한 패킷인 것처럼 타임스탬프를 늘립니다.
const KEEPALIVE_PAYLOAD_TYPE: u8 = 101;
const KEEPALIVE_PAYLOAD: [u8; 4] = [0x00, 0x68, 0x34, 0x00];
const KEEPALIVE_TIMESTAMP_STEP: u32 = 480;

// 빈 송신 보고를 수신 보고 앞에 붙이는 주기입니다.
const SENDER_REPORT_INTERVAL: Duration = Duration::from_secs(5);

// 키프레임 요청의 최소 간격입니다. 서버가 이보다 오래 조용하면 화면이 멈춰 인코딩을 쉬는 중이므로 요청하지 않습니다.
const KEYFRAME_INTERVAL: Duration = Duration::from_millis(250);
const QUIET_AFTER: Duration = Duration::from_millis(1500);

// 프레임이 나오지 않을 때의 조치입니다. 화면이 정지하면 서버가 영상을 보내지 않으므로, 최근에 패킷이 온 경우에만 조치합니다.
const STALL_KEYFRAME: Duration = Duration::from_secs(3);
const STALL_KEYFRAME_REPEAT: Duration = Duration::from_millis(1500);
const STALL_RESETS: [Duration; 2] = [Duration::from_secs(8), Duration::from_secs(15)];

/// 오디오(P)와 비디오(P+1) UDP 소켓입니다. 인증보다 먼저 열어 둡니다.
#[derive(Clone)]
pub struct Sockets {
    pub audio: Arc<UdpSocket>,
    pub video: Arc<UdpSocket>,
    pub server: IpAddr,
    audio_port: u16,
    video_port: u16,
}

impl Sockets {
    pub fn bind(server: IpAddr, port: u16, local: (u16, u16)) -> Result<Self> {
        Ok(Self {
            audio: Arc::new(bind(local.0)?),
            video: Arc::new(bind(local.1)?),
            server,
            audio_port: port,
            video_port: port + 1,
        })
    }

    /// 핸드셰이크가 끝날 때까지 두 소켓에서 서버의 대응 포트로 1바이트를 보냅니다. 서버는 이 패킷을 무시합니다.
    pub async fn punch(self) {
        let mut interval = tokio::time::interval(PUNCH_INTERVAL);

        loop {
            interval.tick().await;
            let _ = self.audio.send_to(&[0], self.audio_address()).await;
            let _ = self.video.send_to(&[0], (self.server, self.video_port)).await;
        }
    }

    // RTCP와 오디오 keepalive는 모두 서버의 오디오 포트로 보냅니다.
    fn audio_address(&self) -> SocketAddr {
        (self.server, self.audio_port).into()
    }
}

/// 비디오와 오디오 수신, RTCP 송신 상태입니다.
pub struct Media {
    sockets: Sockets,
    codec: VideoCodec,
    priority: Priority,
    video_ssrc: u32,
    audio_ssrc: u32,
    receive: SrtpContext,
    audio: SrtpContext,
    // 첫 오디오 앞에 디코더 설정을 알렸는지 여부
    audio_started: bool,
    rtcp: SrtpContext,
    keepalive: SrtpContext,
    streams: Streams,
    assembler: Option<Assembler>,
    keepalive_sequence: u16,
    keepalive_timestamp: u32,
    last_sender_report: Option<Instant>,
    last_keyframe_request: Option<Instant>,
    fir_sequence: u8,
    // 미디어가 시작되지 않았을 때 원인을 가리기 위한 수입니다.
    datagrams: u64,
    authenticated: u64,
    // 재전송을 요청한 순번 수, 이전 조립기들이 메운 패킷 수, 복구하지 못한 손실 수
    nacked: u64,
    filled: u64,
    losses: u64,
    last_packet: Option<Instant>,
    last_frame: Option<Instant>,
    configured: bool,
    stall: Stall,
}

// 프레임 정지 감시 상태입니다. 프레임이 나오면 초기화합니다.
#[derive(Default)]
struct Stall {
    last_request: Option<Instant>,
    resets: usize,
}

impl Media {
    pub fn new(sockets: Sockets, offer: &Offer, codec: VideoCodec, priority: Priority) -> Self {
        Self {
            sockets,
            codec,
            priority,
            video_ssrc: offer.video_ssrc,
            audio_ssrc: offer.audio_ssrc,
            receive: SrtpContext::new(&offer.keys.video_receive),
            audio: SrtpContext::new(&offer.keys.audio_receive),
            audio_started: false,
            rtcp: SrtpContext::new(&offer.keys.video_send),
            keepalive: SrtpContext::new(&offer.keys.audio_send),
            streams: Streams::default(),
            assembler: None,
            keepalive_sequence: 0,
            keepalive_timestamp: 0,
            last_sender_report: None,
            last_keyframe_request: None,
            fir_sequence: 0,
            datagrams: 0,
            authenticated: 0,
            nacked: 0,
            filled: 0,
            losses: 0,
            last_packet: None,
            last_frame: None,
            configured: false,
            stall: Stall::default(),
        }
    }

    /// 비디오 포트로 받은 데이터그램을 처리하고, 내보낼 이벤트를 `events`에 더합니다.
    pub async fn receive(&mut self, datagram: &[u8], from: SocketAddr, now: Instant, events: &mut Vec<Event>) {
        if from.ip() != self.sockets.server {
            return;
        }

        self.datagrams += 1;

        if RtpPacket::is_rtcp(datagram) {
            return;
        }

        let Some(packet) = self
            .receive
            .unprotect_rtp(datagram)
            .as_deref()
            .and_then(RtpPacket::parse)
        else {
            return;
        };

        self.authenticated += 1;
        self.last_packet = Some(now);

        let packets = match self.streams.route(packet, now) {
            Routed::Current(packet) => vec![packet],
            Routed::Switched { ssrc, packets } => {
                // 새 SSRC는 새 디코더 세대입니다. 이전 세대의 참조와 섞이지 않도록 새 설정과 키프레임을 기다립니다.
                self.filled += self.assembler.as_ref().map_or(0, Assembler::filled);
                self.assembler = Some(Assembler::new(self.codec, ssrc, self.priority));
                self.configured = false;
                packets
            }
            Routed::Held => return,
        };

        let target = self.streams.current().unwrap_or_default();

        let Some(assembler) = self.assembler.as_mut() else {
            return;
        };

        let lost: Vec<u16> = packets.into_iter().flat_map(|packet| assembler.push(packet)).collect();

        if !lost.is_empty() {
            self.nacked += lost.len() as u64;
            self.send_rtcp(&rtcp::nack(self.video_ssrc, target, &lost)).await;
        }

        self.drain(now, events).await;
    }

    /// 오디오 포트로 받은 데이터그램을 처리합니다. 같은 포트로 오는 서버 RTCP는 건너뜁니다.
    pub fn receive_audio(&mut self, datagram: &[u8], from: SocketAddr, events: &mut Vec<Event>) {
        if from.ip() != self.sockets.server || RtpPacket::is_rtcp(datagram) {
            return;
        }

        let packet = self.audio.unprotect_rtp(datagram);
        let Some(frame) = packet
            .as_deref()
            .and_then(RtpPacket::parse)
            .and_then(AudioFrame::depacketize)
        else {
            return;
        };

        if !std::mem::replace(&mut self.audio_started, true) {
            events.push(Event::AudioConfig(AudioFrame::CONFIG.to_vec()));
        }

        events.push(Event::Audio(frame));
    }

    /// 조립기에서 꺼낼 수 있는 결과를 모두 꺼냅니다. 재전송 대기 기한이 지났을 때도 부릅니다.
    pub async fn drain(&mut self, now: Instant, events: &mut Vec<Event>) {
        let mut lost = false;

        while let Some(output) = self.assembler.as_mut().and_then(|assembler| assembler.next(now)) {
            match output {
                Assembled::Config(config) => {
                    self.configured = true;
                    events.push(Event::DecoderConfig(config));
                }
                Assembled::Frame(unit) => {
                    self.last_frame = Some(now);
                    self.stall = Stall::default();
                    self.streams.mark_frame(now);
                    events.push(Event::Frame(unit));
                }
                Assembled::Lost => {
                    self.losses += 1;
                    lost = true;
                }
            }
        }

        if lost {
            self.request_keyframe(now).await;
        }
    }

    /// 디코더 설정이나 프레임이 한 번이라도 왔는지 여부
    pub fn started(&self) -> bool {
        self.configured || self.last_frame.is_some()
    }

    /// 마지막으로 프레임을 내보낸 시각
    pub fn last_frame(&self) -> Option<Instant> {
        self.last_frame
    }

    /// 서버가 지금 패킷을 보내고 있는지 여부. 화면이 정지하면 서버는 보내지 않습니다.
    pub fn active(&self, now: Instant) -> bool {
        self.last_packet
            .is_some_and(|last| now.duration_since(last) <= QUIET_AFTER)
    }

    /// 프레임이 나오지 않는지 지켜봅니다. 패킷은 오는데 프레임이 멈추면 키프레임을 요청하고, 오래 가면 디코더를 다시 만들게 합니다.
    ///
    /// - `answered`는 미디어 응답을 받은 시각입니다. 프레임이 한 번도 나오지 않았으면 그때부터 잽니다.
    pub async fn watch(&mut self, now: Instant, answered: Instant, events: &mut Vec<Event>) {
        let since = self.last_frame.unwrap_or(answered);

        if !self.active(now) {
            return;
        }

        let stalled = now.duration_since(since);

        if let Some(&threshold) = STALL_RESETS.get(self.stall.resets)
            && stalled >= threshold
        {
            self.stall.resets += 1;
            events.push(Event::ResetDecoder);
            self.stall.last_request = None;
        }

        let due = self
            .stall
            .last_request
            .is_none_or(|last| now.duration_since(last) >= STALL_KEYFRAME_REPEAT);

        if stalled >= STALL_KEYFRAME && due {
            self.stall.last_request = Some(now);
            self.request_keyframe(now).await;
        }
    }

    pub fn stats(&self, offers: u32) -> MediaStats {
        let ssrc = self.streams.current();

        MediaStats {
            datagrams: self.datagrams,
            authenticated: self.authenticated,
            ssrc,
            highest: ssrc.and_then(|ssrc| self.receive.highest_received(ssrc)),
            candidates: u32::try_from(self.streams.candidates()).unwrap_or(u32::MAX),
            offers,
            nacked: self.nacked,
            filled: self.filled + self.assembler.as_ref().map_or(0, Assembler::filled),
            losses: self.losses,
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.assembler.as_ref()?.deadline()
    }

    /// 0.5초마다 보내는 오디오 keepalive와 수신 보고입니다. 5초마다 빈 송신 보고를 앞에 붙입니다.
    pub async fn tick(&mut self, now: Instant) {
        let keepalive = RtpPacket {
            marker: false,
            payload_type: KEEPALIVE_PAYLOAD_TYPE,
            sequence: self.keepalive_sequence,
            timestamp: self.keepalive_timestamp,
            ssrc: self.audio_ssrc,
            payload: KEEPALIVE_PAYLOAD.to_vec(),
        };

        self.keepalive_sequence = self.keepalive_sequence.wrapping_add(1);
        self.keepalive_timestamp = self.keepalive_timestamp.wrapping_add(KEEPALIVE_TIMESTAMP_STEP);

        if let Some(packet) = self.keepalive.protect_rtp(&keepalive.encode()) {
            let _ = self.sockets.audio.send_to(&packet, self.sockets.audio_address()).await;
        }

        let sources: Vec<(u32, u32)> = self
            .streams
            .current()
            .and_then(|ssrc| Some((ssrc, self.receive.highest_received(ssrc)?)))
            .into_iter()
            .collect();

        let mut compound = Vec::new();

        if self
            .last_sender_report
            .is_none_or(|last| now.duration_since(last) >= SENDER_REPORT_INTERVAL)
        {
            self.last_sender_report = Some(now);
            compound.extend(rtcp::sender_report(self.video_ssrc));
        }

        compound.extend(rtcp::receiver_report(self.video_ssrc, &sources));
        self.send_rtcp(&compound).await;
    }

    /// 키프레임을 요청합니다. 최소 간격 안이거나, 서버가 조용하거나, 아직 스트림이 없으면 보내지 않습니다.
    ///
    /// - 키프레임이 올 때까지 프레임을 버릴지는 정하지 않습니다. 손실 뒤에는 조립기가, 디코더를 새로 만든 뒤에는 네이티브 계층이 정합니다.
    pub async fn request_keyframe(&mut self, now: Instant) {
        let recent = self
            .last_keyframe_request
            .is_some_and(|last| now.duration_since(last) < KEYFRAME_INTERVAL);
        let Some(target) = self.streams.current().filter(|_| !recent && self.active(now)) else {
            return;
        };

        self.last_keyframe_request = Some(now);
        self.fir_sequence = self.fir_sequence.wrapping_add(1);
        self.send_rtcp(&rtcp::keyframe_request(self.video_ssrc, target, self.fir_sequence))
            .await;
    }

    async fn send_rtcp(&mut self, compound: &[u8]) {
        if let Some(packet) = self.rtcp.protect_rtcp(compound) {
            let _ = self.sockets.audio.send_to(&packet, self.sockets.audio_address()).await;
        }
    }
}

fn bind(port: u16) -> Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, None)?;

    // 키프레임 하나가 수백 패킷이므로 수신 버퍼를 크게 잡습니다. 운영체제가 더 작게 잡아도 계속합니다.
    let _ = socket.set_recv_buffer_size(RECEIVE_BUFFER_BYTES);
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddr::from(([0, 0, 0, 0], port)).into())?;

    Ok(UdpSocket::from_std(socket.into())?)
}
