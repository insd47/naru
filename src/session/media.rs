use super::{Event, MediaStats};
use crate::error::Result;
use crate::media::{
    Assembled, AudioFrame, Bandwidth, Group, Priority, Routed, RtpPacket, SrtpContext, Streams, VideoCodec, rtcp,
};
use crate::offer::Offer;
use socket2::{Domain, Socket, Type};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

const RECEIVE_BUFFER_BYTES: usize = 4 << 20;
const PUNCH_INTERVAL: Duration = Duration::from_millis(100);

// Minimum interval between keyframe requests. A server quiet for longer than `QUIET_AFTER` is idling on a still screen, so no request is sent.
// A request still unanswered is repeated at doubling intervals up to `KEYFRAME_BACKOFF_LIMIT`: each answer is the largest
// frame there is, and on a lossy path asking again before it can arrive only adds keyframes that are lost the same way.
const KEYFRAME_INTERVAL: Duration = Duration::from_millis(250);
const KEYFRAME_BACKOFF_LIMIT: Duration = Duration::from_secs(2);
const QUIET_AFTER: Duration = Duration::from_millis(1500);

// While a band recovers from a loss, Apple's client asks for a refresh about once a frame. Each answer is a small
// picture predicted from ones it acknowledged, so asking often costs little.
const REFRESH_INTERVAL: Duration = Duration::from_millis(16);

// Frame stall recovery. The server sends no video for a still screen, so recovery only runs when packets arrived recently.
const STALL_KEYFRAME: Duration = Duration::from_secs(3);
const STALL_KEYFRAME_REPEAT: Duration = Duration::from_millis(1500);
const STALL_RESETS: [Duration; 2] = [Duration::from_secs(8), Duration::from_secs(15)];

/// Audio (P) and video (P+1) UDP sockets, opened before authentication.
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

    /// Sends one byte from each socket to the server's matching port until the handshake ends. The server ignores these packets.
    pub async fn punch(self) {
        let mut interval = tokio::time::interval(PUNCH_INTERVAL);

        loop {
            interval.tick().await;
            let _ = self.audio.send_to(&[0], self.audio_address()).await;
            let _ = self.video.send_to(&[0], self.video_address()).await;
        }
    }

    fn audio_address(&self) -> SocketAddr {
        (self.server, self.audio_port).into()
    }

    fn video_address(&self) -> SocketAddr {
        (self.server, self.video_port).into()
    }
}

/// Video and audio reception and RTCP sending state.
pub struct Media {
    sockets: Sockets,
    codec: VideoCodec,
    priority: Priority,
    video_ssrc: u32,
    audio_ssrc: u32,
    video_receive: SrtpContext,
    audio_receive: SrtpContext,
    // SSRC of the audio stream whose decoder config was announced. The server restarts audio with a new SSRC on resize.
    audio_stream: Option<u32>,
    // Video RTCP goes out with the video key and audio RTCP with the audio key, as the server checks each port with its own.
    video_send: SrtpContext,
    audio_send: SrtpContext,
    streams: Streams,
    group: Option<Group>,
    last_keyframe_request: Option<Instant>,
    last_refresh_request: Option<Instant>,
    // An H.264 unit was lost and no keyframe has arrived since. H.264 recovers only from a keyframe, so requests repeat
    // until one arrives rather than going out once and possibly being swallowed by the rate limit.
    awaiting_keyframe: bool,
    fir_sequence: u8,
    // Keyframe requests sent since the last keyframe arrived
    unanswered: u32,
    // Picture size from the latest decoder config, which refresh requests name
    picture_size: Option<(u16, u16)>,
    // Whether the app's decoder holds the pictures the server may reference. A decoder reset stops the
    // acknowledgments until the next keyframe.
    acknowledging: bool,
    // Counts for diagnosing why media did not start.
    datagrams: u64,
    authenticated: u64,
    largest_bytes: u32,
    // Sequence numbers found missing, packets filled by previous groups, and unrecovered losses
    missing: u64,
    filled: u64,
    losses: u64,
    last_packet: Option<Instant>,
    last_timestamp: u32,
    bandwidth: Bandwidth,
    // When the first receiver feedback went out, the start of its clock
    feedback_started: Option<Instant>,
    last_frame: Option<Instant>,
    configured: bool,
    stall: Stall,
}

// Frame stall watch state, reset whenever a frame comes out.
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
            video_receive: SrtpContext::new(&offer.keys.video_receive),
            audio_receive: SrtpContext::new(&offer.keys.audio_receive),
            audio_stream: None,
            video_send: SrtpContext::new(&offer.keys.video_send),
            audio_send: SrtpContext::new(&offer.keys.audio_send),
            streams: Streams::new(codec.bands()),
            group: None,
            last_keyframe_request: None,
            last_refresh_request: None,
            awaiting_keyframe: false,
            fir_sequence: 0,
            unanswered: 0,
            picture_size: None,
            acknowledging: false,
            datagrams: 0,
            authenticated: 0,
            largest_bytes: 0,
            missing: 0,
            filled: 0,
            losses: 0,
            last_packet: None,
            last_timestamp: 0,
            bandwidth: Bandwidth::default(),
            feedback_started: None,
            last_frame: None,
            configured: false,
            stall: Stall::default(),
        }
    }

    /// Handles a datagram received on the video port and appends outgoing events to `events`.
    pub async fn receive(&mut self, datagram: &[u8], from: SocketAddr, now: Instant, events: &mut Vec<Event>) {
        if from.ip() != self.sockets.server {
            return;
        }

        self.datagrams += 1;
        self.largest_bytes = self
            .largest_bytes
            .max(u32::try_from(datagram.len()).unwrap_or(u32::MAX));

        if RtpPacket::is_rtcp(datagram) {
            return;
        }

        let Some(packet) = self
            .video_receive
            .unprotect_rtp(datagram)
            .as_deref()
            .and_then(RtpPacket::parse)
        else {
            return;
        };

        self.authenticated += 1;
        self.last_packet = Some(now);
        self.last_timestamp = packet.timestamp;
        self.bandwidth.observe(now, packet.timestamp, datagram.len());

        let packets = match self.streams.route(packet, now) {
            Routed::Current(packet) => vec![packet],
            Routed::Switched { base, packets } => {
                // A new group is a new decoder generation. Wait for a new config and keyframe so it never mixes with the previous generation's references.
                self.filled += self.group.as_ref().map_or(0, Group::filled);
                self.group = Some(Group::new(self.codec, base, self.priority));
                self.configured = false;
                self.acknowledging = false;
                packets
            }
            Routed::Held => return,
        };

        let Some(group) = self.group.as_mut() else {
            return;
        };

        let missing = packets.into_iter().map(|packet| group.push(packet)).sum::<u64>();
        self.missing += missing;

        if missing > 0 {
            self.bandwidth.lose(now, missing);
        }
        self.drain(now, events).await;
    }

    /// Handles a datagram received on the audio port, skipping server RTCP that arrives on the same port.
    pub fn receive_audio(&mut self, datagram: &[u8], from: SocketAddr, events: &mut Vec<Event>) {
        if from.ip() != self.sockets.server || RtpPacket::is_rtcp(datagram) {
            return;
        }

        let Some(packet) = self
            .audio_receive
            .unprotect_rtp(datagram)
            .as_deref()
            .and_then(RtpPacket::parse)
        else {
            return;
        };

        let ssrc = packet.ssrc;
        let Some(frame) = AudioFrame::depacketize(packet) else {
            return;
        };

        // A decoder kept across streams stops playing on some platforms, so each new stream gets a fresh decoder.
        if self.audio_stream.replace(ssrc) != Some(ssrc) {
            events.push(Event::AudioConfig(AudioFrame::CONFIG.to_vec()));
        }

        events.push(Event::Audio(frame));
    }

    /// Takes every available output from the group. Also called when the deadline for late packets passes.
    pub async fn drain(&mut self, now: Instant, events: &mut Vec<Event>) {
        while let Some(output) = self.group.as_mut().and_then(|group| group.next(now)) {
            match output {
                Assembled::Config(config) => {
                    self.configured = true;
                    self.picture_size = config
                        .picture_size()
                        .and_then(|(width, height)| Some((u16::try_from(width).ok()?, u16::try_from(height).ok()?)));
                    events.push(Event::DecoderConfig(config));
                }
                Assembled::Frame { unit, donl } => {
                    self.last_frame = Some(now);
                    self.stall = Stall::default();
                    self.streams.mark_frame(now);
                    self.awaiting_keyframe &= !unit.keyframe;

                    if unit.keyframe {
                        self.unanswered = 0;
                        self.acknowledging = true;
                    }

                    // Like Apple's client, every band's pictures are acknowledged. The group lets out only intact ones.
                    if let Some(donl) = donl.filter(|_| self.acknowledging) {
                        self.acknowledge(donl).await;
                    }

                    events.push(Event::Frame(unit));
                }
                Assembled::Present => events.push(Event::Present),
                Assembled::Lost => {
                    self.losses += 1;
                    self.awaiting_keyframe |= self.codec == VideoCodec::H264;
                }
            }
        }

        // Asked on every drain until a keyframe arrives, or until every band is intact again. The intervals space the
        // requests out.
        if self.awaiting_keyframe {
            self.request_keyframe(now).await;
        }

        if self.group.as_ref().is_some_and(Group::recovering) {
            self.request_refresh(now).await;
        }
    }

    /// Whether a decoder config or frame has ever arrived
    pub fn started(&self) -> bool {
        self.configured || self.last_frame.is_some()
    }

    /// When the last frame went out
    pub fn last_frame(&self) -> Option<Instant> {
        self.last_frame
    }

    /// Whether the server is currently sending packets. It stops for a still screen.
    pub fn active(&self, now: Instant) -> bool {
        self.last_packet
            .is_some_and(|last| now.duration_since(last) <= QUIET_AFTER)
    }

    /// Watches for frame stalls. When packets arrive but frames stop, it requests a keyframe, and if that lasts, has the decoder recreated.
    ///
    /// - `answered` is when the media answer arrived. Until the first frame, the stall is measured from then.
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
            self.acknowledging = false;
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
            largest_bytes: self.largest_bytes,
            ssrc,
            highest: ssrc.and_then(|ssrc| self.video_receive.highest_received(ssrc)),
            candidates: u32::try_from(self.streams.candidates()).unwrap_or(u32::MAX),
            offers,
            missing: self.missing,
            filled: self.filled + self.group.as_ref().map_or(0, Group::filled),
            losses: self.losses,
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.group.as_ref()?.deadline()
    }

    /// The receiver reports, sent every 0.5 seconds.
    ///
    /// - As in Apple's client, the video report goes to the video port from the video SSRC, and the audio report to the
    ///   audio port from the audio SSRC. Apple's client sends no RTP, and the server rejects any.
    pub async fn tick(&mut self) {
        // Apple's client reports the top band only.
        let video = self
            .streams
            .current()
            .and_then(|ssrc| Some((ssrc, self.video_receive.highest_received(ssrc)?)));
        let mut report = rtcp::receiver_report(self.video_ssrc, video.as_slice());
        report.extend(rtcp::source_description(self.video_ssrc));
        self.send_video_rtcp(&report).await;

        let audio = self
            .audio_stream
            .and_then(|ssrc| Some((ssrc, self.audio_receive.highest_received(ssrc)?)));
        let mut report = rtcp::receiver_report(self.audio_ssrc, audio.as_slice());
        report.extend(rtcp::source_description(self.audio_ssrc));

        if let Some(packet) = self.audio_send.protect_rtcp(&report) {
            let _ = self.sockets.audio.send_to(&packet, self.sockets.audio_address()).await;
        }
    }

    /// Sends the receiver feedback, every 50 ms once video has arrived, and a refresh request while a band recovers.
    pub async fn feedback(&mut self, now: Instant) {
        let Some(last_packet) = self.last_packet else {
            return;
        };

        let started = *self.feedback_started.get_or_insert(now);
        let feedback = rtcp::Feedback {
            timestamp: self.last_timestamp,
            idle_ms: u16::try_from(now.duration_since(last_packet).as_millis()).unwrap_or(u16::MAX),
            // Only the low 16 bits go out, so the clock wraps about every 64 seconds as Apple's does.
            clock: (now.duration_since(started).as_millis() * 1024 / 1000) as u16,
            received: self.authenticated,
            bandwidth_kbps: self.bandwidth.kbps(now),
        };

        self.send_video_rtcp(&rtcp::rate_feedback(self.video_ssrc, &feedback))
            .await;

        // A still screen sends nothing to drain, so a band that broke on it is asked for here.
        if self.group.as_ref().is_some_and(Group::recovering) {
            self.request_refresh(now).await;
        }
    }

    /// Requests a keyframe, unless within the minimum interval, the server is quiet, or there is no stream yet.
    ///
    /// - It does not decide whether to drop frames until the keyframe. The group decides after a loss, and the native layer after recreating the decoder.
    pub async fn request_keyframe(&mut self, now: Instant) {
        let recent = self
            .last_keyframe_request
            .is_some_and(|last| now.duration_since(last) < self.keyframe_interval());
        let Some(target) = self.streams.current().filter(|_| !recent && self.active(now)) else {
            return;
        };

        // A repeat keeps the FIR sequence number, so it reads as the same request rather than a new one (RFC 5104).
        if self.unanswered == 0 {
            self.fir_sequence = self.fir_sequence.wrapping_add(1);
        }

        self.last_keyframe_request = Some(now);
        self.unanswered += 1;
        let request = rtcp::keyframe_request(self.video_ssrc, target, self.fir_sequence);

        // The audio port is where the server has answered this bundle with keyframes.
        if let Some(packet) = self.video_send.protect_rtcp(&request) {
            let _ = self.sockets.audio.send_to(&packet, self.sockets.audio_address()).await;
        }
    }

    // Asks for a picture that predicts only from intact ones, at most once a frame. It goes out on a still screen too,
    // where nothing else would ever replace the band that broke.
    async fn request_refresh(&mut self, now: Instant) {
        if self
            .last_refresh_request
            .is_some_and(|last| now.duration_since(last) < REFRESH_INTERVAL)
        {
            return;
        }

        // Before the first decoder config the size is not known yet and goes out as 0.
        let size = self.picture_size.unwrap_or((0, 0));
        self.last_refresh_request = Some(now);
        self.send_video_rtcp(&rtcp::refresh_request(self.video_ssrc, size))
            .await;
    }

    // Doubles while requests go unanswered, so a keyframe already on its way can arrive before the next request.
    fn keyframe_interval(&self) -> Duration {
        let doublings = self.unanswered.saturating_sub(1).min(4);
        (KEYFRAME_INTERVAL * 2u32.pow(doublings)).min(KEYFRAME_BACKOFF_LIMIT)
    }

    async fn acknowledge(&mut self, donl: u16) {
        self.send_video_rtcp(&rtcp::ltr_acknowledgment(self.video_ssrc, donl))
            .await;
    }

    async fn send_video_rtcp(&mut self, packet: &[u8]) {
        if let Some(packet) = self.video_send.protect_rtcp(packet) {
            let _ = self.sockets.video.send_to(&packet, self.sockets.video_address()).await;
        }
    }
}

fn bind(port: u16) -> Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, None)?;

    // A keyframe is hundreds of packets, so ask for a large receive buffer. Carry on if the OS grants less.
    let _ = socket.set_recv_buffer_size(RECEIVE_BUFFER_BYTES);
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddr::from(([0, 0, 0, 0], port)).into())?;

    Ok(UdpSocket::from_std(socket.into())?)
}
