use super::RtpPacket;
use super::h264::CONFIG_MARKER;
use super::unit::{AccessUnit, DecoderConfig, VideoCodec};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

// How long an access unit with a sequence gap is held for retransmission. On a LAN it usually arrives within a few ms.
const HOLD: Duration = Duration::from_millis(75);

// Beyond this many buffered packets, everything is treated as lost and dropped.
const MAX_BUFFERED: usize = 4096;

// Most sequence numbers requested for retransmission at once. Larger gaps are recovered with a keyframe request.
const MAX_NACK: u64 = 64;

/// Output of the assembler.
#[derive(Debug, PartialEq)]
pub enum Assembled {
    /// New decoder config. The size may have changed, so reconfigure the decoder.
    Config(DecoderConfig),
    Frame(AccessUnit),
    /// An access unit that couldn't be recovered in time was dropped. Request a keyframe.
    Lost,
}

/// What to favor after an unrecoverable loss. This is the same choice as the Screen Sharing app's quality setting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Enum)]
pub enum Priority {
    /// Drops only the lost unit and keeps emitting frames. Parts of the screen may be corrupted until a keyframe arrives.
    Latency,
    /// Emits nothing until a keyframe arrives. The screen stays on the last intact frame meanwhile.
    #[default]
    Quality,
}

/// Collects one SSRC's RTP packets in sequence order and assembles them into access units.
///
/// - Has no I/O or clock. The caller pushes packets with the current time and calls `next()` again at `deadline()`.
/// - At startup, emits no access units until a decoder config and a keyframe have arrived. Incomplete units are always dropped.
/// - For HEVC, every access unit carrying parameter sets emits that config first and then the unit, the same order as H.264's config packet.
/// - [`Priority`] decides whether units whose reference was lost are emitted after a loss. A buffer overflow always waits for a keyframe.
pub struct Assembler {
    codec: VideoCodec,
    ssrc: u32,
    priority: Priority,
    packets: BTreeMap<u64, RtpPacket>,
    // Next extended sequence number to assemble, and the highest one received so far
    next: Option<u64>,
    highest: Option<u64>,
    stalled_since: Option<Instant>,
    configured: bool,
    awaiting_keyframe: bool,
    // Packets that filled a gap (retransmitted or reordered)
    filled: u64,
    // HEVC unit to emit after its config
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

    /// Packets that have filled a gap so far, for checking whether retransmissions actually arrive
    pub fn filled(&self) -> u64 {
        self.filled
    }

    /// Pushes one packet and returns the sequence numbers of any new gap, which should be requested for retransmission.
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

    /// Takes out one assembled output, or `None` if there is nothing more.
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
                // The next unit isn't complete yet. If the received packets are contiguous, wait for later packets;
                // if there is a gap, wait for retransmission until the deadline.
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

    /// When `next()` must be called again, or `None` when not waiting on a gap
    pub fn deadline(&self) -> Option<Instant> {
        self.stalled_since.map(|since| since + HOLD)
    }

    // Finds the last sequence number of the unit starting at `start`. A config packet is a unit by itself.
    // A packet with the marker bit ends the unit; a timestamp change without a marker also ends it at the previous packet.
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

    // Drops the unit with the gap and resumes after the first marker past the gap. If no marker has arrived yet, keeps waiting.
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

    // A unit was lost. When favoring quality, wait for a keyframe.
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

    // Maps a 16-bit sequence number to the extended sequence number closest to the highest one received.
    fn extend(&self, sequence: u16) -> u64 {
        let Some(highest) = self.highest else {
            return (1 << 32) + u64::from(sequence);
        };

        let candidate = (highest & !0xffff) | u64::from(sequence);

        [candidate - 0x1_0000, candidate, candidate + 0x1_0000]
            .into_iter()
            .min_by_key(|value| value.abs_diff(highest))
            .expect("there is a candidate")
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

    // A config, an IDR in two fragments, and two P frames.
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

        // Sequence number 15 is lost, then 16 (P), 17 (P), and 18 (IDR) arrive.
        assembler.push(packet(16, 9000, true, P_SLICE));
        assembler.push(packet(17, 12000, true, P_SLICE));
        assembler.push(packet(18, 15000, true, IDR));

        assert!(drain(&mut assembler, now).is_empty());

        // 16 is the first marker after the gap, so it ends the damaged unit and is dropped with it. 17 is dropped as a non-keyframe, and output resumes at 18.
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

    // HEVC parameter sets arrive in the same unit as the keyframe, so that unit emits the config first and then the unit.
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
            panic!("the config should come first");
        };
        assert_eq!((config.codec, config.level), (VideoCodec::Hevc, 0x96));

        let Some(Assembled::Frame(keyframe)) = assembler.next(now) else {
            panic!("the keyframe should follow");
        };
        assert!(keyframe.keyframe);
        assert_eq!(keyframe.nals.last().unwrap(), &[0x28, 0x01, 1, 2]);

        assert!(matches!(assembler.next(now), Some(Assembled::Frame(unit)) if !unit.keyframe));
        assert_eq!(assembler.next(now), None);
    }
}
