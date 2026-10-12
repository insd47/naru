use super::RtpPacket;
use super::h264::CONFIG_MARKER;
use super::unit::{AccessUnit, DecoderConfig, VideoCodec};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

// 순번에 구멍이 생긴 접근 단위를 재전송이 올 때까지 붙잡아 두는 시간입니다. LAN에서는 보통 몇 ms 안에 옵니다.
const HOLD: Duration = Duration::from_millis(75);

// 이보다 많은 패킷이 쌓이면 손실로 보고 모두 버립니다.
const MAX_BUFFERED: usize = 4096;

// 한 번에 재전송을 요청하는 순번의 수입니다. 이보다 큰 구멍은 키프레임 요청으로 복구합니다.
const MAX_NACK: u64 = 64;

/// 조립 결과입니다.
#[derive(Debug, PartialEq)]
pub enum Assembled {
    /// 새 디코더 설정. 크기가 바뀌었을 수 있으므로 디코더를 다시 설정합니다.
    Config(DecoderConfig),
    Frame(AccessUnit),
    /// 기한 안에 복구하지 못한 접근 단위를 버렸습니다. 키프레임을 요청해야 합니다.
    Lost,
}

/// 복구하지 못한 손실 뒤에 무엇을 우선할지입니다. Screen Sharing 앱의 화질 설정과 같은 선택입니다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Enum)]
pub enum Priority {
    /// 잃은 단위만 버리고 다음 프레임을 계속 내보냅니다. 키프레임이 올 때까지 화면 일부가 깨질 수 있습니다.
    Latency,
    /// 키프레임이 올 때까지 내보내지 않습니다. 그동안 화면은 마지막 온전한 프레임에 멈춥니다.
    #[default]
    Quality,
}

/// SSRC 하나의 RTP 패킷을 순번대로 모아 접근 단위로 조립합니다.
///
/// - I/O와 시계를 갖지 않습니다. 호출자가 패킷과 현재 시각을 넣고, `deadline()`에 다시 `next()`를 부릅니다.
/// - 시작할 때는 디코더 설정과 키프레임이 올 때까지 접근 단위를 내보내지 않습니다. 불완전한 단위는 언제나 버립니다.
/// - HEVC는 파라미터 세트가 든 접근 단위마다 그 설정을 먼저 내보내고 단위를 이어서 내보냅니다. H.264의 설정 패킷과 같은 순서입니다.
/// - 단위 하나를 잃은 뒤 참조를 잃은 단위를 내보낼지는 [`Priority`]가 정합니다. 버퍼가 넘치면 언제나 키프레임을 기다립니다.
pub struct Assembler {
    codec: VideoCodec,
    ssrc: u32,
    priority: Priority,
    packets: BTreeMap<u64, RtpPacket>,
    // 다음에 조립할 확장 순번과, 지금까지 받은 가장 큰 확장 순번
    next: Option<u64>,
    highest: Option<u64>,
    stalled_since: Option<Instant>,
    configured: bool,
    awaiting_keyframe: bool,
    // 구멍을 메운 패킷 수(재전송이나 순서 바뀜)
    filled: u64,
    // 설정을 먼저 내보내고 이어서 내보낼 HEVC 단위
    pending: Option<AccessUnit>,
}

impl Assembler {
    pub fn new(codec: VideoCodec, ssrc: u32, priority: Priority) -> Self {
        Self {
            codec,
            ssrc,
            priority,
            packets: BTreeMap::new(),
            next: None,
            highest: None,
            stalled_since: None,
            configured: false,
            awaiting_keyframe: true,
            filled: 0,
            pending: None,
        }
    }

    /// 지금까지 구멍을 메운 패킷 수. 재전송이 실제로 오는지 볼 때 씁니다.
    pub fn filled(&self) -> u64 {
        self.filled
    }

    /// 패킷 하나를 넣고, 새로 구멍이 생긴 순번을 반환합니다. 반환한 순번은 재전송을 요청합니다.
    pub fn push(&mut self, packet: RtpPacket) -> Vec<u16> {
        let sequence = self.extend(packet.sequence);

        if self.next.is_some_and(|next| sequence < next) || self.packets.contains_key(&sequence) {
            return Vec::new();
        }

        if self.highest.is_some_and(|highest| sequence < highest) {
            self.filled += 1;
        }

        let missing = match self.highest {
            Some(highest) if sequence > highest + 1 => {
                let first = (highest + 1).max(sequence.saturating_sub(MAX_NACK));
                (first..sequence).map(|lost| lost as u16).collect()
            }
            _ => Vec::new(),
        };

        self.highest = Some(self.highest.map_or(sequence, |highest| highest.max(sequence)));
        self.next.get_or_insert(sequence);
        self.packets.insert(sequence, packet);
        missing
    }

    /// 조립된 결과를 하나 꺼냅니다. 더 꺼낼 것이 없으면 `None`입니다.
    pub fn next(&mut self, now: Instant) -> Option<Assembled> {
        if let Some(unit) = self.pending.take() {
            return Some(Assembled::Frame(unit));
        }

        loop {
            if self.packets.len() > MAX_BUFFERED {
                return Some(self.reset());
            }

            let next = self.next?;

            let Some(end) = self.unit_end(next) else {
                // 다음 단위를 아직 완성할 수 없습니다. 받은 패킷이 빈틈없이 이어져 있으면 뒤의 패킷을 기다리면 되고,
                // 구멍이 있으면 재전송을 기한까지 기다립니다.
                let contiguous = self
                    .highest
                    .is_none_or(|highest| self.packets.len() as u64 == highest + 1 - next);

                if contiguous {
                    self.stalled_since = None;
                    return None;
                }

                let since = *self.stalled_since.get_or_insert(now);

                if now.duration_since(since) < HOLD {
                    return None;
                }

                return self.skip_damaged(next);
            };

            self.stalled_since = None;
            let packets: Vec<RtpPacket> = (next..=end)
                .filter_map(|sequence| self.packets.remove(&sequence))
                .collect();
            self.next = Some(end + 1);

            if let Some(output) = self.emit(packets) {
                return Some(output);
            }
        }
    }

    /// `next()`를 다시 불러야 하는 시각. 구멍을 기다리는 중이 아니면 `None`입니다.
    pub fn deadline(&self) -> Option<Instant> {
        self.stalled_since.map(|since| since + HOLD)
    }

    // `start`에서 시작하는 단위의 마지막 순번을 찾습니다. 설정 패킷은 그 자체로 한 단위입니다.
    // 마커 비트가 있는 패킷이 끝이며, 마커 없이 타임스탬프가 바뀌어도 앞 패킷에서 끝난 것으로 봅니다.
    fn unit_end(&self, start: u64) -> Option<u64> {
        let first = self.packets.get(&start)?;

        if self.codec == VideoCodec::H264 && first.payload.first() == Some(&CONFIG_MARKER) {
            return Some(start);
        }

        let mut sequence = start;

        loop {
            let packet = self.packets.get(&sequence)?;

            if packet.marker {
                return Some(sequence);
            }

            match self.packets.get(&(sequence + 1)) {
                Some(following) if following.timestamp != first.timestamp => return Some(sequence),
                _ => sequence += 1,
            }
        }
    }

    // 구멍이 있는 단위를 버리고, 구멍 뒤의 첫 마커 다음부터 다시 조립합니다. 마커가 아직 없으면 더 기다립니다.
    fn skip_damaged(&mut self, next: u64) -> Option<Assembled> {
        let gap = (next..).find(|sequence| !self.packets.contains_key(sequence))?;
        let marker = self
            .packets
            .range(gap..)
            .find(|(_, packet)| packet.marker)
            .map(|(&sequence, _)| sequence)?;

        self.packets = self.packets.split_off(&(marker + 1));
        self.next = Some(marker + 1);
        self.stalled_since = None;
        Some(self.lose())
    }

    fn reset(&mut self) -> Assembled {
        self.packets.clear();
        self.next = None;
        self.stalled_since = None;
        self.awaiting_keyframe = true;
        Assembled::Lost
    }

    // 단위 하나를 잃었습니다. 화질을 우선하면 키프레임까지 기다립니다.
    fn lose(&mut self) -> Assembled {
        self.awaiting_keyframe |= self.priority == Priority::Quality;
        Assembled::Lost
    }

    fn emit(&mut self, packets: Vec<RtpPacket>) -> Option<Assembled> {
        let first = packets.first()?;

        if self.codec == VideoCodec::H264
            && let Some(config) = DecoderConfig::from_h264(&first.payload)
        {
            self.configured = true;
            return Some(Assembled::Config(config));
        }

        let payloads = packets.iter().map(|packet| packet.payload.as_slice());

        let Some(unit) = AccessUnit::depacketize(self.codec, self.ssrc, first.timestamp, payloads) else {
            return Some(self.lose());
        };

        let config = match self.codec {
            VideoCodec::Hevc => DecoderConfig::from_hevc(&unit.nals),
            VideoCodec::H264 => None,
        };

        if !(self.configured || unit.has_sps()) || (self.awaiting_keyframe && !unit.keyframe) {
            return None;
        }

        self.awaiting_keyframe = false;

        match config {
            Some(config) => {
                self.configured = true;
                self.pending = Some(unit);
                Some(Assembled::Config(config))
            }
            None => Some(Assembled::Frame(unit)),
        }
    }

    // 16비트 순번을 지금까지 받은 가장 큰 순번에 가장 가까운 확장 순번으로 바꿉니다.
    fn extend(&self, sequence: u16) -> u64 {
        let Some(highest) = self.highest else {
            return (1 << 32) + u64::from(sequence);
        };

        let candidate = (highest & !0xffff) | u64::from(sequence);

        [candidate - 0x1_0000, candidate, candidate + 0x1_0000]
            .into_iter()
            .min_by_key(|value| value.abs_diff(highest))
            .expect("후보가 있다")
    }
}

#[cfg(test)]
mod tests {
    use super::{Assembled, Assembler, HOLD, Priority};
    use crate::media::{RtpPacket, VideoCodec};
    use std::time::{Duration, Instant};

    const SSRC: u32 = 7;
    const IDR: &[u8] = &[0x65, 0x88];
    const P_SLICE: &[u8] = &[0x41, 0x9a];

    fn packet(sequence: u16, timestamp: u32, marker: bool, payload: &[u8]) -> RtpPacket {
        RtpPacket {
            marker,
            payload_type: 100,
            sequence,
            timestamp,
            ssrc: SSRC,
            payload: payload.to_vec(),
        }
    }

    fn config() -> Vec<u8> {
        let record = [1, 0x64, 0, 0x34, 0xff, 0xe1, 0, 2, 0x67, 0x64, 1, 0, 1, 0x68];
        let mut packet = vec![0x92, b'a', b'v', b'c', b'1'];
        packet.extend_from_slice(&((record.len() + 8) as u32).to_be_bytes());
        packet.extend_from_slice(b"avcC");
        packet.extend_from_slice(&record);
        packet
    }

    fn drain(assembler: &mut Assembler, now: Instant) -> Vec<Assembled> {
        std::iter::from_fn(|| assembler.next(now)).collect()
    }

    fn frames(outputs: &[Assembled]) -> Vec<(u32, bool)> {
        outputs
            .iter()
            .filter_map(|output| match output {
                Assembled::Frame(unit) => Some((unit.timestamp, unit.keyframe)),
                _ => None,
            })
            .collect()
    }

    // 설정, 두 조각의 IDR, P 프레임 두 장입니다.
    fn stream(start: u16) -> Vec<RtpPacket> {
        let at = |offset: u16| start.wrapping_add(offset);

        vec![
            packet(at(0), 0, false, &config()),
            packet(at(1), 0, false, &[0x7c, 0x85, 0x88]),
            packet(at(2), 0, true, &[0x7c, 0x45, 0x99]),
            packet(at(3), 3000, true, P_SLICE),
            packet(at(4), 6000, true, P_SLICE),
        ]
    }

    #[test]
    fn ordered_stream_yields_config_then_frames() {
        let now = Instant::now();
        let mut assembler = Assembler::new(VideoCodec::H264, SSRC, Priority::Quality);

        for packet in stream(100) {
            assert!(assembler.push(packet).is_empty());
        }

        let outputs = drain(&mut assembler, now);
        assert!(matches!(outputs[0], Assembled::Config(_)));
        assert_eq!(frames(&outputs), [(0, true), (3000, false), (6000, false)]);

        let Assembled::Frame(idr) = &outputs[1] else { panic!() };
        assert_eq!(idr.nals, [vec![0x65, 0x88, 0x99]]);
    }

    #[test]
    fn sequence_wrap_and_reordering_are_absorbed() {
        let now = Instant::now();
        let mut assembler = Assembler::new(VideoCodec::H264, SSRC, Priority::Quality);
        let mut packets = stream(65_533);
        packets.swap(1, 2);
        packets.swap(3, 4);

        for packet in packets {
            assembler.push(packet);
        }

        assert_eq!(
            frames(&drain(&mut assembler, now)),
            [(0, true), (3000, false), (6000, false)]
        );
    }

    #[test]
    fn gap_is_reported_once_for_retransmission_and_filled_in_time() {
        let now = Instant::now();
        let mut assembler = Assembler::new(VideoCodec::H264, SSRC, Priority::Quality);
        let mut packets = stream(10);
        let late = packets.remove(2);

        let missing: Vec<u16> = packets.into_iter().flat_map(|packet| assembler.push(packet)).collect();
        assert_eq!(missing, [12]);

        let outputs = drain(&mut assembler, now);
        assert!(matches!(outputs[..], [Assembled::Config(_)]));
        assert_eq!(assembler.deadline(), Some(now + HOLD));

        assembler.push(late);
        let outputs = drain(&mut assembler, now + Duration::from_millis(10));
        assert_eq!(frames(&outputs), [(0, true), (3000, false), (6000, false)]);
        assert_eq!(assembler.deadline(), None);
    }

    #[test]
    fn unrecovered_gap_drops_the_unit_and_waits_for_a_keyframe() {
        let now = Instant::now();
        let mut assembler = Assembler::new(VideoCodec::H264, SSRC, Priority::Quality);

        for packet in stream(10) {
            assembler.push(packet);
        }

        drain(&mut assembler, now);

        // 순번 15는 사라지고 16(P), 17(P), 18(IDR)이 옵니다.
        assembler.push(packet(16, 9000, true, P_SLICE));
        assembler.push(packet(17, 12000, true, P_SLICE));
        assembler.push(packet(18, 15000, true, IDR));

        assert!(drain(&mut assembler, now).is_empty());

        // 16은 구멍 뒤 첫 마커이므로 손상된 단위의 끝으로 보고 함께 버립니다. 17은 키프레임이 아니라 버리고, 18부터 내보냅니다.
        let outputs = drain(&mut assembler, now + HOLD);
        assert_eq!(outputs[0], Assembled::Lost);
        assert_eq!(frames(&outputs), [(15000, true)]);
    }

    #[test]
    fn latency_priority_keeps_frames_flowing_after_a_loss() {
        let now = Instant::now();
        let mut assembler = Assembler::new(VideoCodec::H264, SSRC, Priority::Latency);

        for packet in stream(10) {
            assembler.push(packet);
        }

        drain(&mut assembler, now);
        assembler.push(packet(16, 9000, true, P_SLICE));
        assembler.push(packet(17, 12000, true, P_SLICE));
        assert!(drain(&mut assembler, now).is_empty());

        let outputs = drain(&mut assembler, now + HOLD);
        assert_eq!(outputs[0], Assembled::Lost);
        assert_eq!(frames(&outputs), [(12000, false)]);
    }

    #[test]
    fn frames_before_the_decoder_config_are_withheld() {
        let now = Instant::now();
        let mut assembler = Assembler::new(VideoCodec::H264, SSRC, Priority::Quality);
        assembler.push(packet(1, 0, true, IDR));
        assert!(drain(&mut assembler, now).is_empty());
    }

    #[test]
    fn late_duplicate_is_ignored() {
        let now = Instant::now();
        let mut assembler = Assembler::new(VideoCodec::H264, SSRC, Priority::Quality);

        for packet in stream(10) {
            assembler.push(packet);
        }

        drain(&mut assembler, now);
        assert!(assembler.push(packet(12, 0, true, &[0x7c, 0x45, 0x99])).is_empty());
        assert!(drain(&mut assembler, now).is_empty());
    }

    // HEVC는 파라미터 세트가 키프레임과 같은 단위로 오므로, 그 단위에서 설정을 먼저 내보내고 단위를 이어서 내보냅니다.
    #[test]
    fn hevc_unit_with_parameter_sets_yields_config_then_frame() {
        const VPS: &[u8] = &[0x40, 0x01, 0x0c];
        const SPS: &[u8] = &[0x42, 0x01, 0x01, 0x04, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x96];
        const PPS: &[u8] = &[0x44, 0x01, 0xc0];
        let mut aggregate = vec![0x60, 0x01];

        for nal in [VPS, SPS, PPS] {
            aggregate.extend_from_slice(&(nal.len() as u16).to_be_bytes());
            aggregate.extend_from_slice(nal);
        }

        let now = Instant::now();
        let mut assembler = Assembler::new(VideoCodec::Hevc, SSRC, Priority::Quality);
        assembler.push(packet(10, 0, false, &aggregate));
        assembler.push(packet(11, 0, false, &[0x62, 0x01, 0x94, 1]));
        assembler.push(packet(12, 0, true, &[0x62, 0x01, 0x54, 2]));
        assembler.push(packet(13, 3000, true, &[0x02, 0x01, 3]));

        let Some(Assembled::Config(config)) = assembler.next(now) else {
            panic!("설정이 먼저 와야 합니다");
        };
        assert_eq!((config.codec, config.level), (VideoCodec::Hevc, 0x96));

        let Some(Assembled::Frame(keyframe)) = assembler.next(now) else {
            panic!("키프레임이 이어져야 합니다");
        };
        assert!(keyframe.keyframe);
        assert_eq!(keyframe.nals.last().unwrap(), &[0x28, 0x01, 1, 2]);

        assert!(matches!(assembler.next(now), Some(Assembled::Frame(unit)) if !unit.keyframe));
        assert_eq!(assembler.next(now), None);
    }
}
