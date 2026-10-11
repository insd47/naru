use super::RtpPacket;
use super::h264;
use super::h264::DecoderConfig;
use std::collections::BTreeMap;
use std::time::Duration;
use std::time::Instant;

// 순번에 구멍이 생긴 접근 단위를 재전송이 올 때까지 붙잡아 두는 시간입니다. LAN에서는 보통 몇 ms 안에 옵니다.
const HOLD: Duration = Duration::from_millis(75);

// 이보다 많은 패킷이 쌓이면 손실로 보고 모두 버립니다.
const MAX_BUFFERED: usize = 4096;

// 한 번에 재전송을 요청하는 순번의 수입니다. 이보다 큰 구멍은 키프레임 요청으로 복구합니다.
const MAX_NACK: u64 = 64;

/// 디코더에 넣을 수 있는 완성된 접근 단위입니다.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessUnit {
    pub ssrc: u32,
    /// RTP 타임스탬프(90 kHz)
    pub timestamp: u32,
    pub keyframe: bool,
    /// 시작 코드 없는 NAL 목록
    pub nals: Vec<Vec<u8>>,
}

/// 조립 결과입니다.
#[derive(Debug, PartialEq)]
pub enum Assembled {
    /// 새 디코더 설정. 크기가 바뀌었을 수 있으므로 디코더를 다시 설정합니다.
    Config(DecoderConfig),
    Frame(AccessUnit),
    /// 기한 안에 복구하지 못한 접근 단위를 버렸습니다. 키프레임을 요청해야 합니다.
    Lost,
}

/// SSRC 하나의 RTP 패킷을 순번대로 모아 접근 단위로 조립합니다.
///
/// - I/O와 시계를 갖지 않습니다. 호출자가 패킷과 현재 시각을 넣고, `deadline()`에 다시 `next()`를 부릅니다.
/// - 손실 뒤에는 키프레임이 올 때까지, 시작할 때는 디코더 설정과 키프레임이 올 때까지 접근 단위를 내보내지 않습니다. 불완전한 단위나 참조를 잃은 단위를 디코더에 넣지 않기 위함입니다.
pub struct Assembler {
    ssrc: u32,
    packets: BTreeMap<u64, RtpPacket>,
    // 다음에 조립할 확장 순번과, 지금까지 받은 가장 큰 확장 순번
    next: Option<u64>,
    highest: Option<u64>,
    stalled_since: Option<Instant>,
    configured: bool,
    awaiting_keyframe: bool,
    // 구멍을 메운 패킷 수(재전송이나 순서 바뀜)
    filled: u64,
}

impl Assembler {
    pub fn new(ssrc: u32) -> Self {
        Self {
            ssrc,
            packets: BTreeMap::new(),
            next: None,
            highest: None,
            stalled_since: None,
            configured: false,
            awaiting_keyframe: true,
            filled: 0,
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

    /// 다음 단위가 키프레임이 될 때까지 내보내지 않습니다. 디코더가 오류를 알렸을 때 씁니다.
    pub fn await_keyframe(&mut self) {
        self.awaiting_keyframe = true;
    }

    // `start`에서 시작하는 단위의 마지막 순번을 찾습니다. 설정 패킷은 그 자체로 한 단위입니다.
    // 마커 비트가 있는 패킷이 끝이며, 마커 없이 타임스탬프가 바뀌어도 앞 패킷에서 끝난 것으로 봅니다.
    fn unit_end(&self, start: u64) -> Option<u64> {
        let first = self.packets.get(&start)?;

        if first.payload.first() == Some(&h264::CONFIG_MARKER) {
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
        self.awaiting_keyframe = true;
        Some(Assembled::Lost)
    }

    fn reset(&mut self) -> Assembled {
        self.packets.clear();
        self.next = None;
        self.stalled_since = None;
        self.awaiting_keyframe = true;
        Assembled::Lost
    }

    fn emit(&mut self, packets: Vec<RtpPacket>) -> Option<Assembled> {
        let first = packets.first()?;

        if let Some(config) = DecoderConfig::parse(&first.payload) {
            self.configured = true;
            return Some(Assembled::Config(config));
        }

        let timestamp = first.timestamp;

        let Some(nals) = h264::depacketize(packets.iter().map(|packet| packet.payload.as_slice())) else {
            self.awaiting_keyframe = true;
            return Some(Assembled::Lost);
        };

        let keyframe = h264::is_keyframe(&nals);
        let ready = self.configured || h264::has_sps(&nals);

        if !ready || (self.awaiting_keyframe && !keyframe) {
            return None;
        }

        self.awaiting_keyframe = false;

        Some(Assembled::Frame(AccessUnit {
            ssrc: self.ssrc,
            timestamp,
            keyframe,
            nals,
        }))
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
    use super::{Assembled, Assembler, HOLD};
    use crate::media::RtpPacket;
    use std::time::Duration;
    use std::time::Instant;

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
        let mut assembler = Assembler::new(SSRC);

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
        let mut assembler = Assembler::new(SSRC);
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
        let mut assembler = Assembler::new(SSRC);
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
        let mut assembler = Assembler::new(SSRC);

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
    fn frames_before_the_decoder_config_are_withheld() {
        let now = Instant::now();
        let mut assembler = Assembler::new(SSRC);
        assembler.push(packet(1, 0, true, IDR));
        assert!(drain(&mut assembler, now).is_empty());
    }

    #[test]
    fn late_duplicate_is_ignored() {
        let now = Instant::now();
        let mut assembler = Assembler::new(SSRC);

        for packet in stream(10) {
            assembler.push(packet);
        }

        drain(&mut assembler, now);
        assert!(assembler.push(packet(12, 0, true, &[0x7c, 0x45, 0x99])).is_empty());
        assert!(drain(&mut assembler, now).is_empty());
    }
}
