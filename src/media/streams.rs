use super::RtpPacket;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

// Conditions for switching to a new group: the current one produced no frame for this long, and the new group sent at least this many packets.
const SWITCH_AFTER: Duration = Duration::from_secs(2);
const MIN_PACKETS: usize = 5;

// Packets held per candidate SSRC. On a switch they are replayed into the assembler so the config packet isn't lost.
const MAX_HELD: usize = 256;

/// Picks which group of SSRCs among the received video packets to decode.
///
/// - Each band comes on its own SSRC, and a group is as many consecutive SSRCs as there are bands, the lowest being the top band.
///   After a resolution change or session switch, the server sends on a new group.
/// - A group is picked once every band has sent a packet, since a band's first picture is the only one that draws it whole.
/// - An abandoned group is never picked again, which prevents flip-flopping while the server briefly sends both.
pub struct Streams {
    bands: u8,
    // Lowest SSRC of the current group
    current: Option<u32>,
    abandoned: HashSet<u32>,
    held: HashMap<u32, Vec<RtpPacket>>,
    last_frame: Option<Instant>,
}

/// Where a packet should go.
#[derive(Debug, PartialEq)]
pub enum Routed {
    /// Packet of the current group
    Current(RtpPacket),
    /// Switched to the group starting at `base`. Start assembling it anew and push `packets` in order.
    Switched { base: u32, packets: Vec<RtpPacket> },
    /// The packet belonged to a candidate SSRC and was held or dropped.
    Held,
}

impl Streams {
    pub fn new(bands: u8) -> Self {
        Self {
            bands,
            current: None,
            abandoned: HashSet::new(),
            held: HashMap::new(),
            last_frame: None,
        }
    }

    pub fn route(&mut self, packet: RtpPacket, now: Instant) -> Routed {
        let ssrc = packet.ssrc;

        if self
            .current
            .is_some_and(|base| ssrc.wrapping_sub(base) < u32::from(self.bands))
        {
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

        if self.current.is_some() && !quiet {
            return Routed::Held;
        }

        let Some(base) = self.complete_group(ssrc) else {
            return Routed::Held;
        };

        if let Some(previous) = self.current {
            self.abandoned.extend(self.members(previous));
        }

        self.current = Some(base);
        self.last_frame = Some(now);

        let packets = self
            .members(base)
            .flat_map(|ssrc| self.held.remove(&ssrc).unwrap_or_default())
            .collect();
        self.held.clear();
        Routed::Switched { base, packets }
    }

    // A held group that `ssrc` belongs to, with every band present and enough packets in all
    fn complete_group(&self, ssrc: u32) -> Option<u32> {
        (0..u32::from(self.bands))
            .map(|offset| ssrc.wrapping_sub(offset))
            .find(|&base| {
                let counts: Option<Vec<usize>> = self
                    .members(base)
                    .map(|ssrc| self.held.get(&ssrc).map(Vec::len))
                    .collect();
                counts.is_some_and(|counts| counts.iter().sum::<usize>() >= MIN_PACKETS)
            })
    }

    fn members(&self, base: u32) -> impl Iterator<Item = u32> + use<> {
        (0..u32::from(self.bands)).map(move |offset| base.wrapping_add(offset))
    }

    /// Reports that the current group produced a frame.
    pub fn mark_frame(&mut self, now: Instant) {
        self.last_frame = Some(now);
    }

    /// Lowest SSRC of the current group, which is the top band
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
        let mut streams = Streams::new(1);
        let routed = feed(&mut streams, 1, MIN_PACKETS as u16, now);

        let Routed::Switched { base: 1, packets } = &routed[MIN_PACKETS - 1] else {
            panic!()
        };

        assert_eq!(packets.len(), MIN_PACKETS);
        assert_eq!(packets[0].sequence, 0);
    }

    // The bands of one instant arrive top first, but a lower SSRC can still arrive last.
    #[test]
    fn group_is_chosen_once_every_band_has_sent() {
        let now = Instant::now();
        let mut streams = Streams::new(4);

        for ssrc in [0xffff_ffff, 1, 2] {
            assert!(
                feed(&mut streams, ssrc, 3, now)
                    .iter()
                    .all(|routed| *routed == Routed::Held)
            );
        }

        let Routed::Switched { base, packets } = streams.route(packet(0, 0), now) else {
            panic!("the last band completes the group")
        };

        assert_eq!(base, 0xffff_ffff);
        assert_eq!(
            packets.iter().map(|packet| packet.ssrc).collect::<Vec<_>>(),
            [0xffff_ffff, 0xffff_ffff, 0xffff_ffff, 0, 1, 1, 1, 2, 2, 2]
        );
        assert_eq!(streams.route(packet(2, 3), now), Routed::Current(packet(2, 3)));
        assert_eq!(streams.current(), Some(0xffff_ffff));
    }

    #[test]
    fn new_ssrc_takes_over_only_after_frames_stop() {
        let now = Instant::now();
        let mut streams = Streams::new(1);
        feed(&mut streams, 1, 5, now);
        streams.mark_frame(now);

        assert!(
            feed(&mut streams, 2, 10, now + SWITCH_AFTER / 2)
                .iter()
                .all(|routed| *routed == Routed::Held)
        );

        let routed = streams.route(packet(2, 10), now + SWITCH_AFTER);
        assert!(matches!(routed, Routed::Switched { base: 2, .. }));
        assert_eq!(streams.current(), Some(2));
    }

    #[test]
    fn abandoned_group_is_never_resumed() {
        let now = Instant::now();
        let mut streams = Streams::new(2);
        feed(&mut streams, 1, 3, now);
        feed(&mut streams, 2, 3, now);
        feed(&mut streams, 8, 3, now + SWITCH_AFTER);
        feed(&mut streams, 9, 3, now + SWITCH_AFTER);
        assert_eq!(streams.current(), Some(8));

        for ssrc in [1, 2] {
            assert!(
                feed(&mut streams, ssrc, 20, now + SWITCH_AFTER * 5)
                    .iter()
                    .all(|routed| *routed == Routed::Held)
            );
        }

        assert_eq!(streams.current(), Some(8));
    }
}
