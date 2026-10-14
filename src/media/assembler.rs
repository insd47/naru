use super::RtpPacket;
use super::h264::CONFIG_MARKER;
use super::sequence;
use super::unit::{AccessUnit, DecoderConfig, VideoCodec};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

// How long an access unit with a sequence gap waits for the missing packets. The server does not retransmit, so only
// packets that arrive out of order fill a gap. Captures of Apple's client on a lossy LAN had none, and on a lossy path
// nearly every picture has a gap, so waiting longer only delays the screen.
const HOLD: Duration = Duration::from_millis(20);

// Beyond this many buffered packets, everything is treated as lost and dropped.
const MAX_BUFFERED: usize = 4096;

/// Output of one band's assembler.
#[derive(Debug, PartialEq)]
pub enum Reassembled {
    /// H.264 decoder config from its config packet
    Config(DecoderConfig),
    /// A complete access unit and its DONL, if the codec carries one
    Unit(AccessUnit, Option<u16>),
    /// An access unit that couldn't be recovered in time was dropped.
    Lost,
    /// Too many packets were buffered, so all of them were dropped.
    Overflow,
}

/// Collects one SSRC's RTP packets in sequence order and assembles them into access units.
///
/// - Has no I/O or clock. The caller pushes packets with the current time and calls `next()` again at `deadline()`.
/// - Emits every complete unit and reports each one it had to drop. Which units reach the decoder is the caller's call.
pub struct Assembler {
    codec: VideoCodec,
    band: u8,
    packets: BTreeMap<u64, RtpPacket>,
    // Next extended sequence number to assemble, and the highest one received so far
    next: Option<u64>,
    highest: Option<u64>,
    stalled_since: Option<Instant>,
    // Packets that filled a gap by arriving out of order
    filled: u64,
}

impl Assembler {
    pub fn new(codec: VideoCodec, band: u8) -> Self {
        Self {
            codec,
            band,
            packets: BTreeMap::new(),
            next: None,
            highest: None,
            stalled_since: None,
            filled: 0,
        }
    }

    /// Packets that have filled a gap so far by arriving out of order
    pub fn filled(&self) -> u64 {
        self.filled
    }

    /// RTP timestamp of the oldest packet not yet assembled, which is a unit still on its way
    pub fn incomplete(&self) -> Option<u32> {
        self.packets.first_key_value().map(|(_, packet)| packet.timestamp)
    }

    /// Pushes one packet and returns how many sequence numbers it showed to be missing.
    pub fn push(&mut self, packet: RtpPacket) -> u64 {
        let sequence = sequence::extend(packet.sequence, self.highest);

        if self.next.is_some_and(|next| sequence < next) || self.packets.contains_key(&sequence) {
            return 0;
        }

        if self.highest.is_some_and(|highest| sequence < highest) {
            self.filled += 1;
        }

        let missing = self.highest.map_or(0, |highest| sequence.saturating_sub(highest + 1));

        self.highest = Some(self.highest.map_or(sequence, |highest| highest.max(sequence)));
        self.next.get_or_insert(sequence);
        self.packets.insert(sequence, packet);
        missing
    }

    /// Takes out one assembled output, or `None` if there is nothing more.
    pub fn next(&mut self, now: Instant) -> Option<Reassembled> {
        if self.packets.len() > MAX_BUFFERED {
            self.packets.clear();
            self.next = None;
            self.stalled_since = None;
            return Some(Reassembled::Overflow);
        }

        let next = self.next?;

        let Some(end) = self.unit_end(next) else {
            // The next unit isn't complete yet. If the received packets are contiguous, wait for later packets;
            // if there is a gap, wait for late packets until the deadline.
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
        Some(self.emit(&packets))
    }

    /// When `next()` must be called again, or `None` when not waiting on a gap
    pub fn deadline(&self) -> Option<Instant> {
        self.stalled_since.map(|since| since + HOLD)
    }

    // Finds the last sequence number of the unit starting at `start`. A config packet is a unit by itself. A packet
    // with the marker bit ends the unit; a timestamp change without a marker also ends it at the previous packet.
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

    // Drops the unit with the gap and resumes after the first marker past the gap. If no marker has arrived yet, keeps
    // waiting.
    fn skip_damaged(&mut self, next: u64) -> Option<Reassembled> {
        let gap = (next..).find(|sequence| !self.packets.contains_key(sequence))?;
        let marker = self
            .packets
            .range(gap..)
            .find(|(_, packet)| packet.marker)
            .map(|(&sequence, _)| sequence)?;

        self.packets = self.packets.split_off(&(marker + 1));
        self.next = Some(marker + 1);
        self.stalled_since = None;
        Some(Reassembled::Lost)
    }

    fn emit(&self, packets: &[RtpPacket]) -> Reassembled {
        let first = &packets[0];

        if self.codec == VideoCodec::H264
            && let Some(config) = DecoderConfig::from_h264(&first.payload)
        {
            return Reassembled::Config(config);
        }

        let payloads = packets.iter().map(|packet| packet.payload.as_slice());

        match AccessUnit::depacketize(self.codec, self.band, first.timestamp, payloads) {
            Some((unit, donl)) => Reassembled::Unit(unit, donl),
            None => Reassembled::Lost,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Assembler, HOLD, Reassembled};
    use crate::media::{RtpPacket, VideoCodec};
    use std::time::{Duration, Instant};

    const SSRC: u32 = 7;
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
        // x264's Baseline profile SPS for 1920×1080, and a PPS
        let record = [
            1, 0x42, 0xc0, 0x28, 0xff, 0xe1, 0, 26, 0x67, 0x42, 0xc0, 0x28, 0xd9, 0x00, 0x78, 0x02, 0x27, 0xe5, 0xc0,
            0x44, 0x00, 0x00, 0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xc8, 0x3c, 0x60, 0xc9, 0x20, 1, 0, 1, 0x68,
        ];
        let mut packet = vec![0x92, b'a', b'v', b'c', b'1'];
        packet.extend_from_slice(&((record.len() + 8) as u32).to_be_bytes());
        packet.extend_from_slice(b"avcC");
        packet.extend_from_slice(&record);
        packet
    }

    fn drain(assembler: &mut Assembler, now: Instant) -> Vec<Reassembled> {
        std::iter::from_fn(|| assembler.next(now)).collect()
    }

    fn units(outputs: &[Reassembled]) -> Vec<(u32, bool)> {
        outputs
            .iter()
            .filter_map(|output| match output {
                Reassembled::Unit(unit, _) => Some((unit.timestamp, unit.keyframe)),
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
    fn ordered_stream_yields_config_then_units() {
        let now = Instant::now();
        let mut assembler = Assembler::new(VideoCodec::H264, 0);

        for packet in stream(100) {
            assert_eq!(assembler.push(packet), 0);
        }

        let outputs = drain(&mut assembler, now);
        assert!(matches!(outputs[0], Reassembled::Config(_)));
        assert_eq!(units(&outputs), [(0, true), (3000, false), (6000, false)]);

        let Reassembled::Unit(idr, None) = &outputs[1] else {
            panic!()
        };
        assert_eq!(idr.nals, [vec![0x65, 0x88, 0x99]]);
    }

    #[test]
    fn sequence_wrap_and_reordering_are_absorbed() {
        let now = Instant::now();
        let mut assembler = Assembler::new(VideoCodec::H264, 0);
        let mut packets = stream(65_533);
        packets.swap(1, 2);
        packets.swap(3, 4);

        for packet in packets {
            assembler.push(packet);
        }

        assert_eq!(
            units(&drain(&mut assembler, now)),
            [(0, true), (3000, false), (6000, false)]
        );
    }

    #[test]
    fn gap_is_counted_once_and_filled_by_a_late_packet_in_time() {
        let now = Instant::now();
        let mut assembler = Assembler::new(VideoCodec::H264, 0);
        let mut packets = stream(10);
        let late = packets.remove(2);

        let missing: u64 = packets.into_iter().map(|packet| assembler.push(packet)).sum();
        assert_eq!(missing, 1);

        let outputs = drain(&mut assembler, now);
        assert!(matches!(outputs[..], [Reassembled::Config(_)]));
        assert_eq!(assembler.deadline(), Some(now + HOLD));
        assert_eq!(assembler.incomplete(), Some(0));

        assembler.push(late);
        let outputs = drain(&mut assembler, now + Duration::from_millis(10));
        assert_eq!(units(&outputs), [(0, true), (3000, false), (6000, false)]);
        assert_eq!(assembler.deadline(), None);
        assert_eq!(assembler.incomplete(), None);
    }

    // 16 is the first marker after the gap, so it ends the damaged unit and is dropped with it.
    #[test]
    fn unrecovered_gap_drops_the_unit_through_the_next_marker() {
        let now = Instant::now();
        let mut assembler = Assembler::new(VideoCodec::H264, 0);

        for packet in stream(10) {
            assembler.push(packet);
        }

        drain(&mut assembler, now);
        assembler.push(packet(16, 9000, true, P_SLICE));
        assembler.push(packet(17, 12000, true, P_SLICE));
        assert!(drain(&mut assembler, now).is_empty());

        let outputs = drain(&mut assembler, now + HOLD);
        assert_eq!(outputs[0], Reassembled::Lost);
        assert_eq!(units(&outputs), [(12000, false)]);
    }

    #[test]
    fn late_duplicate_is_ignored() {
        let now = Instant::now();
        let mut assembler = Assembler::new(VideoCodec::H264, 0);

        for packet in stream(10) {
            assembler.push(packet);
        }

        drain(&mut assembler, now);
        assert_eq!(assembler.push(packet(12, 0, true, &[0x7c, 0x45, 0x99])), 0);
        assert!(drain(&mut assembler, now).is_empty());
    }

    #[test]
    fn hevc_unit_carries_its_band_and_don() {
        let now = Instant::now();
        let mut assembler = Assembler::new(VideoCodec::Hevc, 2);
        assembler.push(packet(10, 0, false, &[0x62, 0x01, 0x81, 0, 9, 1]));
        assembler.push(packet(11, 0, true, &[0x62, 0x01, 0x41, 0, 9, 2]));

        let Some(Reassembled::Unit(unit, Some(9))) = assembler.next(now) else {
            panic!("the unit should come out with its DONL");
        };
        assert_eq!((unit.band, unit.nals), (2, vec![vec![0x02, 0x01, 1, 2]]));
    }
}
