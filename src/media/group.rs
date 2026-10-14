use super::RtpPacket;
use super::assembler::{Assembler, Reassembled};
use super::references::References;
use super::sequence;
use super::unit::{AccessUnit, DecoderConfig, VideoCodec};
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

// How long a unit waits for an earlier one still arriving on another band. A band gives up on a gap by itself, so this
// only bounds a band that went quiet partway through a unit.
const MAX_WAIT: Duration = Duration::from_secs(1);

// How long after a band of an instant the instant counts as complete when no band below it follows. The bands of one
// instant arrive within about 9 ms of each other, measured on Apple's client.
const INSTANT_WAIT: Duration = Duration::from_millis(10);

/// Output of a group.
#[derive(Debug, PartialEq)]
pub enum Assembled {
    /// New decoder config. The size may have changed, so reconfigure the decoder.
    Config(DecoderConfig),
    /// Access unit for the decoder, with its DONL if the codec carries one
    Frame { unit: AccessUnit, donl: Option<u16> },
    /// Every band of one instant has gone out, so the screen they make up can be shown. Only for video in several
    /// bands, and only while every band's latest picture is intact.
    Present,
    /// An access unit that couldn't be recovered in time was dropped. H.264 recovers only from a keyframe.
    Lost,
}

/// Assembles one group of SSRCs, one per band, into access units in decoding order.
///
/// - Has no I/O or clock. The caller pushes packets with the current time and calls `next()` again at `deadline()`.
/// - At startup, emits no access units until a decoder config and a keyframe have arrived. Only the top band carries
///   keyframes, so the other bands' units before it are dropped.
/// - Bands assemble independently, so units are put back in DONL order. A unit waits for an earlier one only while that
///   one is still arriving on its band; the bands of one instant share an RTP timestamp and arrive top first.
/// - A config goes out only when it differs from the last one. Each HEVC band repeats the same parameter sets, which
///   come in the same unit as the keyframe, so that unit emits the config first and then the unit.
/// - With several bands, an instant ends when its bottom band goes out, a band of a later instant goes out, or no band
///   follows for a moment. Showing each band as it decodes would show the top one ahead of the others.
/// - H.264 drops only a lost unit and keeps emitting, so parts of the screen may be corrupted until a keyframe arrives;
///   after a buffer overflow it emits nothing until one does.
/// - HEVC pictures that do not decode intact, because something they predict from was lost, are marked so on the unit
///   and not shown. While any band's latest picture is lost or not intact, the screen is not shown either, so it never
///   pairs bands from different moments; Apple's client likewise shows bands only as a whole screen during losses.
pub struct Group {
    codec: VideoCodec,
    base: u32,
    bands: Vec<Assembler>,
    // Units by extended DONL, and the last one let through
    waiting: BTreeMap<u64, Waiting>,
    released: Option<u64>,
    config: Option<DecoderConfig>,
    awaiting_keyframe: bool,
    // Which pictures decode intact (HEVC only)
    references: Option<References>,
    // Bands whose latest picture was lost or does not decode intact
    broken: Vec<bool>,
    outputs: VecDeque<Assembled>,
    // RTP timestamp of the instant whose bands are going out, and when the last of them went out
    instant: Option<(u32, Instant)>,
}

struct Waiting {
    unit: AccessUnit,
    donl: u16,
    since: Instant,
}

impl Group {
    /// Starts a group whose top band is `base`, with as many bands as the codec takes.
    pub fn new(codec: VideoCodec, base: u32) -> Self {
        let bands: Vec<Assembler> = (0..codec.bands())
            .map(|band| Assembler::new(codec, base.wrapping_add(u32::from(band)), band))
            .collect();
        let references = (codec == VideoCodec::Hevc).then(References::default);

        Self {
            codec,
            base,
            waiting: BTreeMap::new(),
            released: None,
            config: None,
            awaiting_keyframe: true,
            references,
            broken: vec![false; bands.len()],
            bands,
            outputs: VecDeque::new(),
            instant: None,
        }
    }

    /// Packets that have filled a gap so far by arriving out of order
    pub fn filled(&self) -> u64 {
        self.bands.iter().map(Assembler::filled).sum()
    }

    /// Pushes one packet of the group and returns how many sequence numbers it showed to be missing on its band.
    pub fn push(&mut self, packet: RtpPacket) -> u64 {
        let band = packet.ssrc.wrapping_sub(self.base) as usize;
        self.bands.get_mut(band).map_or(0, |assembler| assembler.push(packet))
    }

    /// Takes out one output, or `None` if there is nothing more.
    pub fn next(&mut self, now: Instant) -> Option<Assembled> {
        if self.outputs.is_empty() {
            self.collect(now);
        }

        self.outputs.pop_front()
    }

    /// When `next()` must be called again, or `None` when nothing is waiting
    pub fn deadline(&self) -> Option<Instant> {
        let waiting = self.waiting.values().map(|waiting| waiting.since + MAX_WAIT).min();
        let instant = self.instant.map(|(_, since)| since + INSTANT_WAIT);
        self.bands
            .iter()
            .filter_map(Assembler::deadline)
            .chain(waiting)
            .chain(instant)
            .min()
    }

    fn collect(&mut self, now: Instant) {
        for band in 0..self.bands.len() {
            while let Some(output) = self.bands[band].next(now) {
                match output {
                    Reassembled::Config(config) => self.announce(config),
                    Reassembled::Unit(unit, None) => self.admit(unit, None, now),
                    Reassembled::Unit(unit, Some(donl)) => self.queue(unit, donl, now),
                    Reassembled::Lost => self.lose(band, false),
                    Reassembled::Overflow => self.lose(band, true),
                }
            }
        }

        self.release(now);

        if self
            .instant
            .is_some_and(|(_, since)| now.duration_since(since) >= INSTANT_WAIT)
        {
            self.end_instant();
        }
    }

    // A unit arriving after a later one already went ahead without it can no longer be decoded in order, so it counts
    // as lost.
    fn queue(&mut self, unit: AccessUnit, donl: u16, now: Instant) {
        let highest = self.waiting.last_key_value().map(|(&key, _)| key).max(self.released);
        let key = sequence::extend(donl, highest);

        if self.released.is_some_and(|released| key <= released) {
            self.lose(usize::from(unit.band), false);
            return;
        }

        self.waiting.insert(key, Waiting { unit, donl, since: now });
    }

    fn release(&mut self, now: Instant) {
        while let Some((&key, waiting)) = self.waiting.first_key_value() {
            let in_order = self.released.is_some_and(|released| key == released + 1);

            if !in_order && self.preceded(&waiting.unit) && now.duration_since(waiting.since) < MAX_WAIT {
                break;
            }

            let waiting = self.waiting.remove(&key).expect("the first key is present");
            self.released = Some(key);
            self.admit(waiting.unit, Some(waiting.donl), now);
        }
    }

    // Whether a band is still assembling a unit that decodes before this one: an earlier instant, or the same instant
    // higher up.
    fn preceded(&self, unit: &AccessUnit) -> bool {
        self.bands.iter().enumerate().any(|(band, assembler)| {
            assembler.incomplete().is_some_and(|timestamp| {
                let earlier = unit.timestamp.wrapping_sub(timestamp).cast_signed() > 0;
                earlier || (timestamp == unit.timestamp && band < usize::from(unit.band))
            })
        })
    }

    fn admit(&mut self, mut unit: AccessUnit, donl: Option<u16>, now: Instant) {
        if !(self.config.is_some() || unit.has_sps()) || (self.awaiting_keyframe && !unit.keyframe) {
            return;
        }

        self.awaiting_keyframe = false;

        if self.codec == VideoCodec::Hevc
            && let Some(config) = DecoderConfig::from_hevc(&unit.nals)
        {
            self.announce(config);
        }

        // A picture whose headers cannot be read counts as intact, so headers the parser does not know never stop the
        // screen.
        let intact = self
            .references
            .as_mut()
            .is_none_or(|references| references.intact(&unit.nals).unwrap_or(true));

        unit.intact = intact;
        self.broken[usize::from(unit.band)] = !intact;

        if self.bands.len() == 1 {
            self.outputs.push_back(Assembled::Frame { unit, donl });
            return;
        }

        if self.instant.is_some_and(|(timestamp, _)| timestamp != unit.timestamp) {
            self.end_instant();
        }

        let bottom = usize::from(unit.band) + 1 == self.bands.len();
        self.instant = Some((unit.timestamp, now));
        self.outputs.push_back(Assembled::Frame { unit, donl });

        if bottom {
            self.end_instant();
        }
    }

    // Instants let out together, after a gap held them back, show only the latest screen, as Apple's client resumes
    // from the newest picture rather than replaying the ones in between.
    fn end_instant(&mut self) {
        if self.instant.take().is_some() && !self.broken.contains(&true) {
            self.outputs.retain(|output| *output != Assembled::Present);
            self.outputs.push_back(Assembled::Present);
        }
    }

    fn announce(&mut self, config: DecoderConfig) {
        if self.config.as_ref() != Some(&config) {
            self.config = Some(config.clone());
            self.outputs.push_back(Assembled::Config(config));
        }
    }

    // A unit of `band` was lost. HEVC keeps going even after an overflow: its references tell which pictures are broken,
    // the screen waits for the band to be intact, and the server answers with pictures predicted from acknowledged
    // ones, which waiting for a keyframe would throw away.
    fn lose(&mut self, band: usize, overflow: bool) {
        self.awaiting_keyframe |= overflow && self.references.is_none();
        self.broken[band] = true;

        self.outputs.push_back(Assembled::Lost);
    }
}

#[cfg(test)]
mod tests {
    use super::{Assembled, Group, INSTANT_WAIT, MAX_WAIT};
    use crate::media::{RtpPacket, VideoCodec};
    use std::time::Instant;

    const IDR: &[u8] = &[0x65, 0x88];
    const P_SLICE: &[u8] = &[0x41, 0x9a];

    // HEVC parameter sets as an aggregation packet with DONL `donl`. `level` tells configs apart.
    fn parameter_sets(donl: u16, level: u8) -> Vec<u8> {
        let vps: &[u8] = &[0x40, 0x01, 0x0c];
        let sps: &[u8] = &[0x42, 0x01, 0x01, 0x04, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, level];
        let pps: &[u8] = &[0x44, 0x01, 0xc0];
        let mut packet = vec![0x60, 0x01];
        packet.extend_from_slice(&donl.to_be_bytes());

        for nal in [vps, sps, pps] {
            packet.extend_from_slice(&(nal.len() as u16).to_be_bytes());
            packet.extend_from_slice(nal);
        }

        packet
    }

    // A single-NAL HEVC slice: an IDR (type 19) when `keyframe`, otherwise TRAIL_R (type 1).
    fn slice(donl: u16, keyframe: bool) -> Vec<u8> {
        let header = if keyframe { 0x26 } else { 0x02 };
        let mut packet = vec![header, 0x01];
        packet.extend_from_slice(&donl.to_be_bytes());
        packet.push(0xaf);
        packet
    }

    fn packet(ssrc: u32, sequence: u16, timestamp: u32, marker: bool, payload: &[u8]) -> RtpPacket {
        RtpPacket {
            marker,
            payload_type: 100,
            sequence,
            timestamp,
            ssrc,
            payload: payload.to_vec(),
        }
    }

    fn drain(group: &mut Group, now: Instant) -> Vec<Assembled> {
        std::iter::from_fn(|| group.next(now)).collect()
    }

    // (band, DONL) of each frame, with `C` marking a config
    fn summary(outputs: &[Assembled]) -> Vec<String> {
        outputs
            .iter()
            .map(|output| match output {
                Assembled::Config(_) => "C".into(),
                Assembled::Frame { unit, donl } => format!("{}:{}", unit.band, donl.unwrap_or_default()),
                Assembled::Lost => "L".into(),
                Assembled::Present => "|".into(),
            })
            .collect()
    }

    fn h264_config() -> Vec<u8> {
        let record = [1, 0x64, 0, 0x34, 0xff, 0xe1, 0, 2, 0x67, 0x64, 1, 0, 1, 0x68];
        let mut packet = vec![0x92, b'a', b'v', b'c', b'1'];
        packet.extend_from_slice(&((record.len() + 8) as u32).to_be_bytes());
        packet.extend_from_slice(b"avcC");
        packet.extend_from_slice(&record);
        packet
    }

    #[test]
    fn h264_frames_before_the_decoder_config_are_withheld() {
        let now = Instant::now();
        let mut group = Group::new(VideoCodec::H264, 7);
        group.push(packet(7, 1, 0, true, IDR));
        assert!(drain(&mut group, now).is_empty());

        group.push(packet(7, 2, 3000, false, &h264_config()));
        group.push(packet(7, 3, 3000, true, IDR));
        group.push(packet(7, 4, 6000, false, &h264_config()));
        group.push(packet(7, 5, 6000, true, IDR));
        assert_eq!(summary(&drain(&mut group, now)), ["C", "0:0", "0:0"]);
    }

    #[test]
    fn h264_frames_keep_flowing_after_a_loss() {
        let now = Instant::now();
        let mut group = Group::new(VideoCodec::H264, 7);
        group.push(packet(7, 10, 0, false, &h264_config()));
        group.push(packet(7, 11, 0, true, IDR));
        drain(&mut group, now);

        group.push(packet(7, 13, 3000, true, P_SLICE));
        group.push(packet(7, 14, 6000, true, P_SLICE));
        assert!(drain(&mut group, now).is_empty());

        let deadline = group.deadline().unwrap();
        let outputs = drain(&mut group, deadline);
        assert_eq!(outputs[0], Assembled::Lost);
        assert!(matches!(&outputs[1..], [Assembled::Frame { unit, .. }] if unit.timestamp == 6000));
    }

    // Each band repeats the parameter sets, but the decoder hears about them once, and again only when they change.
    #[test]
    fn bands_come_out_in_decoding_order_from_the_top_keyframe() {
        let now = Instant::now();
        let mut group = Group::new(VideoCodec::Hevc, 100);

        // Band 1 of an earlier instant is dropped: nothing decodes before the keyframe.
        group.push(packet(101, 0, 0, true, &slice(9, false)));

        group.push(packet(100, 0, 90, false, &parameter_sets(10, 1)));
        group.push(packet(100, 1, 90, true, &slice(10, true)));
        group.push(packet(101, 1, 90, false, &parameter_sets(11, 1)));
        group.push(packet(101, 2, 90, true, &slice(11, false)));
        group.push(packet(103, 0, 90, false, &parameter_sets(13, 2)));
        group.push(packet(103, 1, 90, true, &slice(13, false)));

        // Band 2 of the same instant is still arriving, so band 3 waits for it.
        group.push(packet(102, 0, 90, false, &parameter_sets(12, 1)));
        assert_eq!(summary(&drain(&mut group, now)), ["C", "0:10", "1:11"]);

        group.push(packet(102, 1, 90, true, &slice(12, false)));
        assert_eq!(summary(&drain(&mut group, now)), ["2:12", "C", "3:13", "|"]);
    }

    // A unit that never arrives does not hold the others back once its band has nothing in flight.
    #[test]
    fn missing_don_is_skipped_when_no_band_is_still_assembling() {
        let now = Instant::now();
        let mut group = Group::new(VideoCodec::Hevc, 100);
        group.push(packet(100, 0, 90, false, &parameter_sets(10, 1)));
        group.push(packet(100, 1, 90, true, &slice(10, true)));
        group.push(packet(102, 0, 90, true, &slice(12, false)));
        assert_eq!(summary(&drain(&mut group, now)), ["C", "0:10", "2:12"]);

        // DONL 11 shows up after 12 went ahead.
        group.push(packet(101, 0, 90, true, &slice(11, false)));
        assert_eq!(summary(&drain(&mut group, now)), ["L"]);
    }

    #[test]
    fn a_band_that_stops_partway_holds_the_others_only_so_long() {
        let now = Instant::now();
        let mut group = Group::new(VideoCodec::Hevc, 100);
        group.push(packet(100, 0, 90, false, &parameter_sets(10, 1)));
        group.push(packet(100, 1, 90, true, &slice(10, true)));
        group.push(packet(101, 0, 90, false, &[0x62, 0x01, 0x81, 0, 11, 1]));
        group.push(packet(102, 0, 90, true, &slice(12, false)));
        assert_eq!(summary(&drain(&mut group, now)), ["C", "0:10"]);
        assert_eq!(summary(&drain(&mut group, now + INSTANT_WAIT)), ["|"]);
        assert_eq!(group.deadline(), Some(now + MAX_WAIT));
        assert_eq!(summary(&drain(&mut group, now + MAX_WAIT)), ["2:12"]);
    }

    // The top band of the next instant ends the one before, and an instant whose lower bands did not change ends on its
    // own.
    #[test]
    fn instants_end_at_the_next_instant_or_after_a_moment() {
        let now = Instant::now();
        let mut group = Group::new(VideoCodec::Hevc, 100);
        group.push(packet(100, 0, 90, false, &parameter_sets(10, 1)));
        group.push(packet(100, 1, 90, true, &slice(10, true)));
        group.push(packet(101, 0, 90, true, &slice(11, false)));
        group.push(packet(100, 2, 180, true, &slice(12, false)));
        assert_eq!(summary(&drain(&mut group, now)), ["C", "0:10", "1:11", "|", "0:12"]);
        assert_eq!(group.deadline(), Some(now + INSTANT_WAIT));
        assert_eq!(summary(&drain(&mut group, now + INSTANT_WAIT)), ["|"]);
        assert_eq!(group.deadline(), None);
    }

    // A band whose picture was lost holds the screen until it has an intact picture again, while the others go on
    // decoding.
    #[test]
    fn a_lost_band_holds_the_screen_until_it_is_intact_again() {
        let now = Instant::now();
        let mut group = Group::new(VideoCodec::Hevc, 100);
        group.push(packet(100, 0, 90, false, &parameter_sets(10, 1)));
        group.push(packet(100, 1, 90, true, &slice(10, true)));
        group.push(packet(101, 0, 90, true, &slice(11, false)));
        assert_eq!(summary(&drain(&mut group, now)), ["C", "0:10", "1:11"]);

        // Band 1 skips a sequence number and gives up on the unit it fell in.
        group.push(packet(101, 2, 180, true, &slice(13, false)));
        let mut later = now;
        let mut outputs = Vec::new();

        while let Some(deadline) = group.deadline() {
            later = deadline;
            outputs.extend(drain(&mut group, later));
        }

        assert_eq!(summary(&outputs), ["|", "L"]);

        group.push(packet(100, 2, 270, true, &slice(14, false)));
        group.push(packet(100, 3, 360, true, &slice(15, false)));
        assert_eq!(summary(&drain(&mut group, later)), ["0:14", "0:15"]);

        group.push(packet(101, 3, 360, true, &slice(16, false)));
        assert_eq!(summary(&drain(&mut group, later)), ["1:16"]);
        assert_eq!(summary(&drain(&mut group, later + INSTANT_WAIT)), ["|"]);
    }
}
