use super::RtpPacket;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

// Conditions for switching to a new SSRC: the current stream produced no frame for this long, and the new SSRC sent at least this many packets.
const SWITCH_AFTER: Duration = Duration::from_secs(2);
const MIN_PACKETS: usize = 5;

// Packets held per candidate SSRC. On a switch they are replayed into the assembler so the config packet isn't lost.
const MAX_HELD: usize = 256;

/// Picks which SSRC of the received video packets to decode.
///
/// - One tile is requested, so one stream is one SSRC. After a resolution change or session switch, the server sends on a new SSRC.
/// - An abandoned SSRC is never picked again, which prevents flip-flopping while the server briefly sends both streams.
#[derive(Default)]
pub struct Streams {
    current: Option<u32>,
    abandoned: HashSet<u32>,
    held: HashMap<u32, Vec<RtpPacket>>,
    last_frame: Option<Instant>,
}

/// Where a packet should go.
#[derive(Debug, PartialEq)]
pub enum Routed {
    /// Packet of the current stream
    Current(RtpPacket),
    /// Switched to this SSRC. Create a new assembler and push `packets` in order.
    Switched { ssrc: u32, packets: Vec<RtpPacket> },
    /// The packet belonged to a candidate SSRC and was held or dropped.
    Held,
}

impl Streams {
    pub fn route(&mut self, packet: RtpPacket, now: Instant) -> Routed {
        let ssrc = packet.ssrc;

        if self.current == Some(ssrc) {
            return Routed::Current(packet);
        }

        if self.abandoned.contains(&ssrc) {
            return Routed::Held;
        }

        let held = self.held.entry(ssrc).or_default();

        if held.len() < MAX_HELD {
            held.push(packet);
        }

        let quiet = self
            .last_frame
            .is_none_or(|last| now.duration_since(last) >= SWITCH_AFTER);

        if held.len() < MIN_PACKETS || (self.current.is_some() && !quiet) {
            return Routed::Held;
        }

        self.abandoned.extend(self.current);
        self.current = Some(ssrc);
        self.last_frame = Some(now);

        let packets = self.held.remove(&ssrc).unwrap_or_default();
        self.held.clear();
        Routed::Switched { ssrc, packets }
    }

    /// Reports that the current stream produced a frame.
    pub fn mark_frame(&mut self, now: Instant) {
        self.last_frame = Some(now);
    }

    pub fn current(&self) -> Option<u32> {
        self.current
    }

    /// Number of other SSRCs being watched as switch candidates
    pub fn candidates(&self) -> usize {
        self.held.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{MIN_PACKETS, Routed, SWITCH_AFTER, Streams};
    use crate::media::RtpPacket;
    use std::time::Instant;

    fn packet(ssrc: u32, sequence: u16) -> RtpPacket {
        RtpPacket {
            marker: false,
            payload_type: 100,
            sequence,
            timestamp: 0,
            ssrc,
            payload: vec![0x41],
        }
    }

    fn feed(streams: &mut Streams, ssrc: u32, count: u16, now: Instant) -> Vec<Routed> {
        (0..count)
            .map(|sequence| streams.route(packet(ssrc, sequence), now))
            .collect()
    }

    #[test]
    fn first_ssrc_is_chosen_with_its_held_packets() {
        let now = Instant::now();
        let mut streams = Streams::default();
        let routed = feed(&mut streams, 1, MIN_PACKETS as u16, now);

        let Routed::Switched { ssrc: 1, packets } = &routed[MIN_PACKETS - 1] else {
            panic!()
        };

        assert_eq!(packets.len(), MIN_PACKETS);
        assert_eq!(packets[0].sequence, 0);
    }

    #[test]
    fn new_ssrc_takes_over_only_after_frames_stop() {
        let now = Instant::now();
        let mut streams = Streams::default();
        feed(&mut streams, 1, 5, now);
        streams.mark_frame(now);

        assert!(
            feed(&mut streams, 2, 10, now + SWITCH_AFTER / 2)
                .iter()
                .all(|routed| *routed == Routed::Held)
        );

        let routed = streams.route(packet(2, 10), now + SWITCH_AFTER);
        assert!(matches!(routed, Routed::Switched { ssrc: 2, .. }));
        assert_eq!(streams.current(), Some(2));
    }

    #[test]
    fn abandoned_ssrc_is_never_resumed() {
        let now = Instant::now();
        let mut streams = Streams::default();
        feed(&mut streams, 1, 5, now);
        feed(&mut streams, 2, 5, now + SWITCH_AFTER);

        assert!(
            feed(&mut streams, 1, 20, now + SWITCH_AFTER * 5)
                .iter()
                .all(|routed| *routed == Routed::Held)
        );
        assert_eq!(streams.current(), Some(2));
    }
}
