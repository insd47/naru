use std::collections::VecDeque;
use std::time::{Duration, Instant};

// Lateness up to this is ordinary jitter, which the apps absorb; only more raises the delay.
const JITTER: Duration = Duration::from_millis(20);
// Arrivals later than this go out at once. A screen this far behind is worse than one that skips.
const MAX_DELAY: Duration = Duration::from_millis(500);
// The delay is dropped this long after the last late arrival. Wi-Fi that pauses for AWDL does so about every 0.52 s.
const CALM: Duration = Duration::from_secs(2);
// How far back the earliest arrival is looked for, which also follows drift between the Mac's clock and this one
const WINDOW: Duration = Duration::from_secs(10);

/// A stream paced by its own RTP clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// 24 kHz
    Video,
    /// 48 kHz
    Audio,
}

impl Lane {
    fn rate(self) -> i64 {
        match self {
            Self::Video => 24_000,
            Self::Audio => 48_000,
        }
    }
}

/// Paces media to the Mac's clock, so pictures and sound held back by the network go out at their own spacing rather
/// than at once.
///
/// - Has no I/O or clock, like the group. The caller pushes items with the current time and takes them out with
///   `next()`, calling it again at `deadline()`.
/// - An item arriving later than the earliest one did, by more than ordinary jitter, raises a delay shared by both lanes,
///   so what follows waits that long and sound stays with the picture. Apple's client does the same when Wi-Fi pauses:
///   late frames show in order, a beat behind, instead of skipping.
/// - Once arrivals have been on time for a while, the delay drops at once and whatever waits goes out, so the screen
///   skips ahead and sound cuts once instead of drifting back.
/// - Items without a timestamp, such as a decoder config, go out with the item before them. Nothing is reordered.
pub struct Playout<T> {
    video: Pace<T>,
    audio: Pace<T>,
    delay: Duration,
    // When an arrival was last late
    late: Option<Instant>,
}

struct Pace<T> {
    rate: i64,
    // Arrival and RTP timestamp of the first item, which offsets count from
    origin: Option<(Instant, u32)>,
    // Each arrival's offset (arrival less RTP time, in microseconds from the origin) that no later one undercut, oldest
    // first, so the front is the earliest within the window
    earliest: VecDeque<(Instant, i64)>,
    queue: VecDeque<(Instant, T)>,
    last_due: Option<Instant>,
}

impl<T> Playout<T> {
    pub fn new() -> Self {
        Self {
            video: Pace::new(Lane::Video),
            audio: Pace::new(Lane::Audio),
            delay: Duration::ZERO,
            late: None,
        }
    }

    /// Queues an item that arrived `now`, paced by its RTP `timestamp` or, without one, by the item before it.
    pub fn push(&mut self, lane: Lane, item: T, timestamp: Option<u32>, now: Instant) {
        self.calm(now);
        let pace = self.lane(lane);

        let due = match timestamp {
            None => pace.last_due.unwrap_or(now),
            Some(timestamp) => {
                let (lateness, on_time) = pace.observe(timestamp, now);

                if lateness > JITTER {
                    self.late = Some(now);
                }

                if lateness > self.delay + JITTER {
                    self.delay = lateness.min(MAX_DELAY);
                }

                on_time + self.delay
            }
        };

        let pace = self.lane(lane);
        let due = pace.last_due.map_or(due, |last| due.max(last));
        pace.last_due = Some(due);
        pace.queue.push_back((due, item));
    }

    /// Takes out an item whose time has come, or `None` if there is none.
    pub fn next(&mut self, now: Instant) -> Option<T> {
        self.calm(now);

        [&mut self.video, &mut self.audio]
            .into_iter()
            .find(|pace| pace.queue.front().is_some_and(|(due, _)| *due <= now))
            .and_then(|pace| pace.queue.pop_front())
            .map(|(_, item)| item)
    }

    /// When the next item is due, or `None` when nothing waits
    pub fn deadline(&self) -> Option<Instant> {
        [&self.video, &self.audio]
            .into_iter()
            .filter_map(|pace| pace.queue.front().map(|(due, _)| *due))
            .min()
    }

    /// The current delay, for diagnostics
    pub fn delay(&self) -> Duration {
        self.delay
    }

    /// Starts a lane's clock anew, for a new stream with timestamps of its own. The delay stays, since the network
    /// didn't change.
    pub fn restart(&mut self, lane: Lane) {
        let pace = self.lane(lane);
        pace.origin = None;
        pace.earliest.clear();
    }

    fn lane(&mut self, lane: Lane) -> &mut Pace<T> {
        match lane {
            Lane::Video => &mut self.video,
            Lane::Audio => &mut self.audio,
        }
    }

    // Drops the delay once arrivals have been on time for a while, sending out what waits.
    fn calm(&mut self, now: Instant) {
        if self.delay.is_zero() || self.late.is_some_and(|late| now.duration_since(late) < CALM) {
            return;
        }

        self.delay = Duration::ZERO;

        for pace in [&mut self.video, &mut self.audio] {
            for (due, _) in &mut pace.queue {
                *due = (*due).min(now);
            }

            pace.last_due = pace.last_due.map(|last| last.min(now));
        }
    }
}

impl<T> Pace<T> {
    fn new(lane: Lane) -> Self {
        Self {
            rate: lane.rate(),
            origin: None,
            earliest: VecDeque::new(),
            queue: VecDeque::new(),
            last_due: None,
        }
    }

    // Returns how late an arrival is against the earliest one in the window, and when it would have arrived on time.
    fn observe(&mut self, timestamp: u32, now: Instant) -> (Duration, Instant) {
        let (origin, origin_timestamp) = *self.origin.get_or_insert((now, timestamp));
        let media = i64::from(timestamp.wrapping_sub(origin_timestamp) as i32) * 1_000_000 / self.rate;
        let offset = now.duration_since(origin).as_micros() as i64 - media;

        while self.earliest.back().is_some_and(|&(_, earlier)| earlier >= offset) {
            self.earliest.pop_back();
        }

        self.earliest.push_back((now, offset));

        while self
            .earliest
            .front()
            .is_some_and(|&(at, _)| now.duration_since(at) > WINDOW)
        {
            self.earliest.pop_front();
        }

        let earliest = self.earliest.front().map_or(offset, |&(_, earliest)| earliest);
        let lateness = Duration::from_micros((offset - earliest) as u64);
        (lateness, now - lateness)
    }
}

#[cfg(test)]
mod tests {
    use super::{CALM, Lane, MAX_DELAY, Playout};
    use std::time::{Duration, Instant};

    // One 60 fps frame at 24 kHz
    const FRAME: u32 = 400;
    const FRAME_TIME: Duration = Duration::from_micros(16_667);

    // Frames arriving every frame time, with the given ones held and arriving together `held` later
    fn arrivals(start: Instant, frames: u32, held: impl Fn(u32) -> Option<Duration>) -> Vec<(Instant, u32)> {
        (0..frames)
            .map(|frame| {
                let sent = start + FRAME_TIME * frame;
                (held(frame).map_or(sent, |late| sent + late), frame * FRAME)
            })
            .collect()
    }

    // Runs the arrivals in time order and returns when each frame went out.
    fn play(playout: &mut Playout<u32>, mut arrivals: Vec<(Instant, u32)>) -> Vec<(Instant, u32)> {
        arrivals.sort_by_key(|&(at, _)| at);
        let mut out = Vec::new();
        let mut pending = arrivals.into_iter().peekable();

        loop {
            let next_arrival = pending.peek().map(|&(at, _)| at);
            let Some(now) = [next_arrival, playout.deadline()].into_iter().flatten().min() else {
                return out;
            };

            while pending.peek().is_some_and(|&(at, _)| at <= now) {
                let (_, timestamp) = pending.next().unwrap();
                playout.push(Lane::Video, timestamp / FRAME, Some(timestamp), now);
            }

            while let Some(frame) = playout.next(now) {
                out.push((now, frame));
            }
        }
    }

    #[test]
    fn steady_frames_go_out_on_arrival() {
        let start = Instant::now();
        let mut playout = Playout::new();
        let out = play(&mut playout, arrivals(start, 120, |_| None));

        assert!(out.iter().all(|&(at, frame)| at == start + FRAME_TIME * frame));
        assert_eq!(playout.delay(), Duration::ZERO);
    }

    // Wi-Fi pausing 100 ms every 0.52 s, as AWDL did: the frames sent during a pause arrive together at its end.
    #[test]
    fn pauses_are_spread_after_the_first() {
        let start = Instant::now();
        let pause = |frame: u32| {
            let sent = FRAME_TIME * frame;
            let into = sent.as_micros() % 520_000;
            (into < 100_000).then(|| Duration::from_micros((100_000 - into) as u64))
        };
        let mut playout = Playout::new();
        let out = play(&mut playout, arrivals(start, 180, pause));

        assert!(
            out.windows(2).all(|pair| pair[0].1 + 1 == pair[1].1),
            "frames out of order"
        );
        let after_first = out
            .iter()
            .skip_while(|&&(at, _)| at < start + Duration::from_millis(600));
        let gaps: Vec<Duration> = after_first
            .clone()
            .zip(after_first.skip(1))
            .map(|(a, b)| b.0 - a.0)
            .collect();
        assert!(
            gaps.iter().all(|gap| *gap <= FRAME_TIME * 2),
            "gaps after the first pause: {gaps:?}"
        );
        assert!(playout.delay() >= Duration::from_millis(80));
    }

    // The first frame to arrive was itself late: the ones after it count from the earliest, and none wait.
    #[test]
    fn a_late_first_frame_holds_nothing_back() {
        let start = Instant::now();
        let early = Duration::from_millis(50);
        let mut arrivals = vec![(start, 0)];
        arrivals.extend((4..60).map(|frame| (start + FRAME_TIME * frame - early, frame * FRAME)));
        let mut playout = Playout::new();
        let out = play(&mut playout, arrivals);

        assert!(
            out.iter()
                .skip(1)
                .all(|&(at, frame)| at == start + FRAME_TIME * frame - early)
        );
        assert_eq!(playout.delay(), Duration::ZERO);
    }

    #[test]
    fn the_delay_drops_once_arrivals_are_on_time() {
        let start = Instant::now();
        let mut playout = Playout::new();
        let frames = (CALM.as_millis() as u32 + 1500) * 60 / 1000;
        let out = play(
            &mut playout,
            arrivals(start, frames, |frame| {
                (frame == 30).then_some(Duration::from_millis(100))
            }),
        );

        assert_eq!(playout.delay(), Duration::ZERO);
        let (at, frame) = *out.last().unwrap();
        assert_eq!(at, start + FRAME_TIME * frame);
    }

    // The Mac's clock running 0.01% slow against this one builds 60 ms of apparent lateness over ten minutes, which the
    // window follows instead of holding frames back.
    #[test]
    fn clock_drift_adds_no_delay() {
        let start = Instant::now();
        let frames = 60 * 600;
        let arrivals = (0..frames)
            .map(|frame| (start + FRAME_TIME * frame + FRAME_TIME * frame / 10_000, frame * FRAME))
            .collect();
        let mut playout = Playout::new();
        play(&mut playout, arrivals);

        assert_eq!(playout.delay(), Duration::ZERO);
    }

    #[test]
    fn far_late_frames_go_out_at_once() {
        let start = Instant::now();
        let mut playout = Playout::new();
        playout.push(Lane::Video, 0, Some(0), start);
        let late = start + FRAME_TIME + MAX_DELAY * 2;
        playout.push(Lane::Video, 1, Some(FRAME), late);

        assert_eq!(playout.delay(), MAX_DELAY);
        assert_eq!(playout.next(late), Some(0));
        assert_eq!(playout.next(late), Some(1));
    }

    // Frames held back arrive together and go out a frame apart; an item without a timestamp waits for the one before it.
    #[test]
    fn untimed_items_keep_their_place() {
        let start = Instant::now();
        let burst = start + Duration::from_millis(120);
        let mut playout = Playout::new();
        playout.push(Lane::Video, 0, Some(0), start);
        playout.push(Lane::Video, 1, Some(FRAME), burst);
        playout.push(Lane::Video, 2, Some(FRAME * 2), burst);
        playout.push(Lane::Video, 3, None, burst);

        assert_eq!(playout.next(burst), Some(0));
        assert_eq!(playout.next(burst), Some(1));
        assert_eq!(playout.next(burst), None);

        let second = playout.deadline().unwrap();
        assert_eq!(second, burst + FRAME_TIME);
        assert_eq!(playout.next(second), Some(2));
        assert_eq!(playout.next(second), Some(3));
    }

    // Sound waits as long as the picture, so the two stay together.
    #[test]
    fn audio_shares_the_video_delay() {
        let start = Instant::now();
        let burst = start + Duration::from_millis(120);
        let mut playout = Playout::new();
        playout.push(Lane::Video, 0, Some(0), start);
        playout.push(Lane::Video, 1, Some(FRAME), burst);
        while playout.next(burst).is_some() {}

        let sound = burst + Duration::from_millis(5);
        playout.push(Lane::Audio, 2, Some(0), sound);

        assert_eq!(playout.next(sound), None);
        assert_eq!(playout.deadline(), Some(sound + playout.delay()));
    }
}
