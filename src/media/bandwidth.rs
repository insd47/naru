//! The bandwidth the receiver reports in its rate feedback, estimated from how video arrives.
//!
//! - The signal is the trend of one-way delay, as in WebRTC's delay-based estimate: for each frame, how much longer its
//!   first packet took to arrive than its RTP timestamp advanced, accumulated and fitted to a line. A queue filling up
//!   tilts the line upward. A fixed delay, a change in how long the Mac takes to encode, and random loss do not, so none
//!   of them lowers the estimate. Apple's client ignores the same things.
//! - Once the queue is full its delay stops rising, and the packets it drops show instead. More than 25% of packets lost
//!   in the last second counts as congestion too. Apple's client kept its estimate through 10% random loss and lowered
//!   it at 30% and more behind a bottleneck. Above 2% the estimate does not climb, as in WebRTC, since a full queue
//!   still drops a few packets after the Mac has slowed down to nearly its rate.
//! - The first packet of a frame, because the Mac spreads a frame's packets over half the frame interval, so later
//!   packets arrive later the bigger the frame even with no queue.
//! - While the delay keeps rising, the estimate falls by 15% every 450 ms, but not below 85% of what arrived in the last
//!   second. Once it stops rising the estimate holds for 4 seconds, then climbs back linearly to the ceiling.
//! - The numbers follow Apple's client behind a queueing bottleneck: near 85% of the bottleneck within 2.5 to 5.5
//!   seconds, and back at the ceiling within 10 seconds after it clears.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

const CEILING_KBPS: f64 = 60000.0;
// The video RTP clock in ticks per millisecond. Every band of one instant shares a timestamp.
const CLOCK_TICKS_PER_MS: f64 = 24.0;
const RATE_WINDOW: Duration = Duration::from_secs(1);
const CONGESTED_LOSS: f64 = 0.25;
const HOLDING_LOSS: f64 = 0.02;

// The trend line and its adaptive threshold take WebRTC's defaults: a line through the last 20 frames of the smoothed
// accumulated delay, scaled by the number of frames seen (up to 60) and a gain of 4, against a threshold that starts at
// 12.5 and follows the scaled trend, quickly downward and slowly upward.
const TREND_FRAMES: usize = 20;
const SMOOTHING: f64 = 0.9;
const TREND_GAIN: f64 = 4.0;
const TREND_FRAMES_CAP: f64 = 60.0;
const THRESHOLD_START: f64 = 12.5;
const THRESHOLD_RANGE: (f64, f64) = (6.0, 600.0);
const THRESHOLD_UP: f64 = 0.0087;
const THRESHOLD_DOWN: f64 = 0.039;
// Frames rising above the threshold in a row before the delay counts as rising, about 170 ms. A queue filling up keeps
// rising, while a sudden step in encoding time tilts the line only briefly.
const RISING_FRAMES: u32 = 10;
// A gap this long (a still screen) starts the trend over, since the queue has drained in the meantime.
const RESTART_AFTER: Duration = Duration::from_secs(1);

// The Mac moves its bitrate one step per 2 seconds, so the estimate falls in spaced steps rather than all at once.
const DECREASE: f64 = 0.85;
const DECREASE_INTERVAL: Duration = Duration::from_millis(450);
const HOLD: Duration = Duration::from_secs(4);
const INCREASE_KBPS_PER_SEC: f64 = 6500.0;

/// Estimates the bandwidth to report from the video packets received.
///
/// - Has no clock of its own. The caller passes the arrival time of each packet and the time of each report.
pub struct Bandwidth {
    // The RTP timestamp extended past its wrap, and the latest frame's timestamp and first arrival
    timestamp: Option<(u32, i64)>,
    frame: Option<(i64, Instant)>,
    // Starts with the first frame and again after a long gap
    trend: Option<Trend>,
    received: VecDeque<(Instant, usize)>,
    received_bytes: usize,
    lost: VecDeque<(Instant, u64)>,
    lost_packets: u64,
    last_rising: Option<Instant>,
    last_decrease: Option<Instant>,
    last_report: Option<Instant>,
    estimate_kbps: f64,
}

impl Bandwidth {
    /// Takes a video packet of `bytes` bytes that arrived at `now`.
    pub fn observe(&mut self, now: Instant, timestamp: u32, bytes: usize) {
        self.received.push_back((now, bytes));
        self.received_bytes += bytes;

        let timestamp = self.extend(timestamp);

        // Later packets of a frame, and late packets of an earlier one, say nothing more about the queue.
        if self.frame.is_some_and(|(current, _)| timestamp <= current) {
            return;
        }

        match self.frame.replace((timestamp, now)) {
            Some((previous, arrived)) if now.duration_since(arrived) < RESTART_AFTER => {
                let delta_ms = now.duration_since(arrived).as_secs_f64() * 1000.0
                    - (timestamp - previous) as f64 / CLOCK_TICKS_PER_MS;

                if self.trend.as_mut().is_some_and(|trend| trend.push(delta_ms, now)) {
                    self.last_rising = Some(now);
                }
            }
            _ => self.trend = Some(Trend::new(now)),
        }
    }

    /// Takes `count` video packets found missing at `now`.
    pub fn lose(&mut self, now: Instant, count: u64) {
        self.lost.push_back((now, count));
        self.lost_packets += count;
    }

    /// The bandwidth to report now, in kbps. Called once per report.
    pub fn kbps(&mut self, now: Instant) -> u16 {
        let elapsed = self
            .last_report
            .replace(now)
            .map_or(Duration::ZERO, |last| now.duration_since(last));

        while let Some(&(at, bytes)) = self.received.front()
            && now.duration_since(at) > RATE_WINDOW
        {
            self.received.pop_front();
            self.received_bytes -= bytes;
        }

        while let Some(&(at, count)) = self.lost.front()
            && now.duration_since(at) > RATE_WINDOW
        {
            self.lost.pop_front();
            self.lost_packets -= count;
        }

        let expected = self.lost_packets + self.received.len() as u64;
        let loss = if expected == 0 {
            0.0
        } else {
            self.lost_packets as f64 / expected as f64
        };

        if loss > CONGESTED_LOSS {
            self.last_rising = Some(now);
        }

        let since_rising = self.last_rising.map(|last| now.duration_since(last));

        // Rising within the last report period: the queue is still filling.
        if since_rising.is_some_and(|since| since <= elapsed.max(Duration::from_millis(50))) {
            if self
                .last_decrease
                .is_none_or(|last| now.duration_since(last) >= DECREASE_INTERVAL)
            {
                self.last_decrease = Some(now);
                let received_kbps = self.received_bytes as f64 * 8.0 / 1000.0 / RATE_WINDOW.as_secs_f64();
                let lowered = (self.estimate_kbps * DECREASE).max(received_kbps * DECREASE);
                self.estimate_kbps = self.estimate_kbps.min(lowered);
            }
        } else if since_rising.is_none_or(|since| since >= HOLD) && loss <= HOLDING_LOSS {
            self.estimate_kbps = (self.estimate_kbps + INCREASE_KBPS_PER_SEC * elapsed.as_secs_f64()).min(CEILING_KBPS);
        }

        self.estimate_kbps as u16
    }

    // Extends the 32-bit RTP timestamp so that it keeps rising across its wrap.
    fn extend(&mut self, timestamp: u32) -> i64 {
        let extended = match self.timestamp {
            Some((last, extended)) => extended + i64::from(timestamp.wrapping_sub(last) as i32),
            None => i64::from(timestamp),
        };

        if self.timestamp.is_none_or(|(_, last)| extended > last) {
            self.timestamp = Some((timestamp, extended));
        }

        extended
    }
}

impl Default for Bandwidth {
    fn default() -> Self {
        Self {
            timestamp: None,
            frame: None,
            trend: None,
            received: VecDeque::new(),
            received_bytes: 0,
            lost: VecDeque::new(),
            lost_packets: 0,
            last_rising: None,
            last_decrease: None,
            last_report: None,
            estimate_kbps: CEILING_KBPS,
        }
    }
}

// The delay trend since the last restart
struct Trend {
    started: Instant,
    frames: usize,
    accumulated_ms: f64,
    smoothed_ms: f64,
    points: VecDeque<(f64, f64)>,
    slope: f64,
    threshold: f64,
    threshold_updated: Instant,
    rising_frames: u32,
}

impl Trend {
    fn new(now: Instant) -> Self {
        Self {
            started: now,
            frames: 0,
            accumulated_ms: 0.0,
            smoothed_ms: 0.0,
            points: VecDeque::new(),
            slope: 0.0,
            threshold: THRESHOLD_START,
            threshold_updated: now,
            rising_frames: 0,
        }
    }

    // Adds one frame's delay change and returns whether the delay is rising.
    fn push(&mut self, delta_ms: f64, now: Instant) -> bool {
        self.frames += 1;
        self.accumulated_ms += delta_ms;
        self.smoothed_ms = SMOOTHING * self.smoothed_ms + (1.0 - SMOOTHING) * self.accumulated_ms;
        self.points.push_back((
            now.duration_since(self.started).as_secs_f64() * 1000.0,
            self.smoothed_ms,
        ));

        if self.points.len() > TREND_FRAMES {
            self.points.pop_front();
        }

        if self.points.len() < TREND_FRAMES {
            return false;
        }

        let previous_slope = std::mem::replace(&mut self.slope, slope(&self.points));
        let scaled = (self.frames as f64).min(TREND_FRAMES_CAP) * self.slope * TREND_GAIN;

        let rising = if scaled > self.threshold && self.slope >= previous_slope {
            self.rising_frames += 1;
            self.rising_frames >= RISING_FRAMES
        } else {
            self.rising_frames = 0;
            false
        };

        self.adapt(scaled, now);
        rising
    }

    // Moves the threshold toward the scaled trend, ignoring spikes far above it.
    fn adapt(&mut self, scaled: f64, now: Instant) {
        let elapsed_ms = (now.duration_since(self.threshold_updated).as_secs_f64() * 1000.0).min(100.0);
        self.threshold_updated = now;

        if scaled.abs() > self.threshold + 15.0 {
            return;
        }

        let rate = if scaled.abs() < self.threshold {
            THRESHOLD_DOWN
        } else {
            THRESHOLD_UP
        };
        self.threshold += rate * (scaled.abs() - self.threshold) * elapsed_ms;
        self.threshold = self.threshold.clamp(THRESHOLD_RANGE.0, THRESHOLD_RANGE.1);
    }
}

// Least-squares slope of the points
fn slope(points: &VecDeque<(f64, f64)>) -> f64 {
    let count = points.len() as f64;
    let (x_mean, y_mean) = points
        .iter()
        .fold((0.0, 0.0), |(x, y), &(px, py)| (x + px / count, y + py / count));
    let (numerator, denominator) = points.iter().fold((0.0, 0.0), |(n, d), &(x, y)| {
        (n + (x - x_mean) * (y - y_mean), d + (x - x_mean) * (x - x_mean))
    });

    if denominator == 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}

#[cfg(test)]
mod tests {
    use super::{Bandwidth, CLOCK_TICKS_PER_MS};
    use std::time::{Duration, Instant};

    const FRAME: Duration = Duration::from_micros(16_667);
    const REPORT: Duration = Duration::from_millis(50);
    const PACKET_BYTES: usize = 1200;

    // Plays `seconds` of 60 fps video of `frame_bytes` per frame through a link of `link_mbps` with a queue of
    // `queue_packets`, dropping `loss` of the packets at random before the link, and reports every 50 ms. Returns the
    // last report.
    struct Path {
        bandwidth: Bandwidth,
        start: Instant,
        clock: Duration,
        frame: u32,
        // When the link finishes sending what it already holds
        busy_until: Duration,
        next_report: Duration,
        seed: u32,
        reports: Vec<(Duration, u16)>,
    }

    impl Path {
        fn new() -> Self {
            Self {
                bandwidth: Bandwidth::default(),
                start: Instant::now(),
                clock: Duration::ZERO,
                frame: 0,
                busy_until: Duration::ZERO,
                next_report: Duration::ZERO,
                seed: 7,
                reports: Vec::new(),
            }
        }

        fn run(&mut self, seconds: f64, link_mbps: f64, delay: Duration, loss: f64, frame_bytes: usize) -> u16 {
            let end = self.clock + Duration::from_secs_f64(seconds);
            let per_packet = Duration::from_secs_f64(PACKET_BYTES as f64 * 8.0 / (link_mbps * 1e6));

            while self.clock < end {
                let timestamp = (f64::from(self.frame) * CLOCK_TICKS_PER_MS * 1000.0 / 60.0) as u32;

                let packets = frame_bytes / PACKET_BYTES;

                for index in 0..packets {
                    // The Mac spreads a frame's packets over the frame interval.
                    let sent = self.clock + FRAME * index as u32 / packets.max(1) as u32;

                    // Tail drop: a packet that would wait behind more than 50 others is lost. The receiver finds a
                    // loss when the next packet arrives, about when this one would have.
                    let queued = self.busy_until.saturating_sub(sent);

                    if self.random() < loss || queued > per_packet * 50 {
                        self.report_until(sent + queued + delay);
                        self.bandwidth.lose(self.start + sent + queued + delay, 1);
                        continue;
                    }

                    self.busy_until = self.busy_until.max(sent) + per_packet;
                    let arrival = self.start + self.busy_until + delay;
                    self.report_until(self.busy_until + delay);
                    self.bandwidth.observe(arrival, timestamp, PACKET_BYTES);
                }

                self.frame += 1;
                self.clock += FRAME;
            }

            self.report_until(self.clock);
            self.reports.last().map_or(0, |&(_, kbps)| kbps)
        }

        fn report_until(&mut self, until: Duration) {
            while self.next_report <= until {
                let kbps = self.bandwidth.kbps(self.start + self.next_report);
                self.reports.push((self.next_report, kbps));
                self.next_report += REPORT;
            }
        }

        // When the reports first fell below `kbps`, after `since`
        fn first_below(&self, since: Duration, kbps: u16) -> Option<Duration> {
            self.reports
                .iter()
                .find(|&&(at, value)| at >= since && value < kbps)
                .map(|&(at, _)| at - since)
        }

        fn random(&mut self) -> f64 {
            self.seed ^= self.seed << 13;
            self.seed ^= self.seed >> 17;
            self.seed ^= self.seed << 5;
            f64::from(self.seed) / f64::from(u32::MAX)
        }
    }

    // 58 Mbps of video, as the Mac sends at its highest step
    const FRAME_BYTES: usize = 58_000_000 / 8 / 60;

    #[test]
    fn a_clear_path_reports_the_ceiling() {
        assert_eq!(Path::new().run(10.0, 1000.0, Duration::ZERO, 0.0, FRAME_BYTES), 60000);
    }

    #[test]
    fn random_loss_does_not_lower_the_estimate() {
        let mut path = Path::new();
        path.run(5.0, 1000.0, Duration::ZERO, 0.0, FRAME_BYTES);
        path.run(10.0, 1000.0, Duration::ZERO, 0.1, FRAME_BYTES);

        assert!(path.reports.iter().all(|&(_, kbps)| kbps == 60000));
    }

    #[test]
    fn a_fixed_delay_does_not_lower_the_estimate() {
        let mut path = Path::new();
        path.run(15.0, 1000.0, Duration::from_millis(50), 0.0, FRAME_BYTES);

        assert!(path.reports.iter().all(|&(_, kbps)| kbps == 60000));
    }

    // A still screen encodes faster than a moving one, so the delay steps up when motion starts. This lowered the
    // estimate when it was judged by the delay itself rather than its trend. Encoding time varied by under 4 ms from
    // frame to frame in Apple's captures, and steps of 15 ms pass.
    #[test]
    fn a_step_in_encoding_time_does_not_lower_the_estimate() {
        let mut path = Path::new();
        path.run(5.0, 1000.0, Duration::ZERO, 0.0, FRAME_BYTES);
        path.run(10.0, 1000.0, Duration::from_millis(15), 0.0, FRAME_BYTES);

        assert!(path.reports.iter().all(|&(_, kbps)| kbps == 60000));
    }

    #[test]
    fn a_bottleneck_lowers_the_estimate_near_its_rate_within_seconds() {
        let mut path = Path::new();
        path.run(5.0, 1000.0, Duration::ZERO, 0.0, FRAME_BYTES);
        let start = path.clock;
        let settled = path.run(10.0, 30.0, Duration::ZERO, 0.0, FRAME_BYTES);

        let below = path
            .first_below(start, 30000)
            .expect("the estimate falls below the bottleneck");
        assert!(below <= Duration::from_secs(6), "fell below 30 Mbps after {below:?}");
        assert!((20000..=30000).contains(&settled), "settled at {settled}");
    }

    #[test]
    fn the_estimate_holds_then_returns_to_the_ceiling_after_the_bottleneck_clears() {
        let mut path = Path::new();
        path.run(5.0, 1000.0, Duration::ZERO, 0.0, FRAME_BYTES);
        path.run(10.0, 30.0, Duration::ZERO, 0.0, FRAME_BYTES);
        let low = path.reports.last().unwrap().1;

        assert_eq!(path.run(3.5, 1000.0, Duration::ZERO, 0.0, FRAME_BYTES), low);
        assert_eq!(path.run(8.0, 1000.0, Duration::ZERO, 0.0, FRAME_BYTES), 60000);
    }

    #[test]
    fn a_still_screen_keeps_the_estimate() {
        let mut path = Path::new();
        path.run(5.0, 1000.0, Duration::ZERO, 0.0, FRAME_BYTES);

        assert_eq!(path.run(10.0, 1000.0, Duration::ZERO, 0.0, 0), 60000);
    }

    #[test]
    fn the_timestamp_extends_across_its_wrap() {
        let mut bandwidth = Bandwidth::default();
        let now = Instant::now();
        bandwidth.observe(now, u32::MAX - 10, 1);
        bandwidth.observe(now, 5, 1);

        assert_eq!(bandwidth.timestamp, Some((5, i64::from(u32::MAX) + 6)));
    }
}
