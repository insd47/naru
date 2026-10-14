use crate::offer::{Answer, Offer};
use std::time::{Duration, Instant};

// An answer without a size means the server's encoder is not ready yet, so the same offer is sent again. Nothing is
// resent before an answer arrives. The server creates a stream per offer, so resending early spawns several streams and
// breaks the start.
const RETRY: Duration = Duration::from_millis(200);
const ATTEMPTS: u32 = 16;

// How long to wait before resending when no answer arrives at all, and how many times. Answers normally take about 1.5
// seconds.
const TIMEOUT: Duration = Duration::from_secs(4);
const SILENT_ATTEMPTS: u32 = 3;

// Minimum interval between offers resent for keyframes after negotiation, so they do not overlap before the server
// opens a new stream. Requests within the interval are deferred.
const REOFFER_INTERVAL: Duration = Duration::from_secs(2);

/// Sends the media offer and waits for an answer with a size. It returns the messages to send; the caller sends them.
///
/// - An answer without a size triggers a resend shortly after; no answer at all triggers one every [`TIMEOUT`]. It
///   gives up past the attempt limits.
/// - The count of silent timeouts restarts whenever an answer arrives.
/// - Offers are also resent after negotiation (pixel size changes, keyframes while the screen is still). The server
///   opens a new stream and sends a config and keyframe.
/// - Resends for keyframes are spaced out; resizes are always sent.
pub struct Negotiation {
    offer: Offer,
    sent: Instant,
    // Offers sent so far, and silent timeouts since the last answer
    offers: u32,
    silent: u32,
    // When to resend after an answer without a size
    retry: Option<Instant>,
    answered: Option<Instant>,
    // A keyframe resend deferred because it fell within the interval
    deferred: bool,
}

impl Negotiation {
    pub fn new(offer: Offer) -> Self {
        Self {
            offer,
            sent: Instant::now(),
            offers: 0,
            silent: 0,
            retry: None,
            answered: None,
            deferred: false,
        }
    }

    /// The first offer
    pub fn start(&mut self, now: Instant) -> Vec<u8> {
        self.send(now)
    }

    /// When the answer with a size arrived, or `None` while still negotiating
    pub fn answered(&self) -> Option<Instant> {
        self.answered
    }

    /// Number of offers sent so far, including resends after negotiation
    pub fn offers(&self) -> u32 {
        self.offers
    }

    /// When to resend, or `None` once negotiated with no deferred resend
    pub fn deadline(&self) -> Option<Instant> {
        match self.answered {
            None => Some(self.retry.unwrap_or(self.sent + TIMEOUT)),
            Some(_) => self.deferred.then_some(self.sent + REOFFER_INTERVAL),
        }
    }

    /// Looks for an answer in a body carried by a framebuffer update, returning `true` for the first answer with a
    /// size.
    pub fn answer(&mut self, body: &[u8], now: Instant) -> bool {
        let Some(answer) = Answer::find(body).filter(|_| self.answered.is_none()) else {
            return false;
        };

        self.silent = 0;

        if answer.ready {
            self.answered = Some(now);
            return true;
        }

        self.retry.get_or_insert(now + RETRY);
        false
    }

    /// Called once [`Self::deadline`] passes. Returns the offer to resend, or `None` when negotiation ran out of
    /// attempts.
    pub fn expire(&mut self, now: Instant) -> Option<Vec<u8>> {
        if self.answered.is_some() {
            return Some(self.send(now));
        }

        if self.retry.is_none() {
            self.silent += 1;
        }

        if self.offers >= ATTEMPTS || self.silent >= SILENT_ATTEMPTS {
            return None;
        }

        Some(self.send(now))
    }

    /// Resends the offer to get a keyframe. Returns `None` while still negotiating; if an offer just went out, it is
    /// deferred to [`Self::deadline`].
    pub fn reoffer(&mut self, now: Instant) -> Option<Vec<u8>> {
        self.answered?;

        if now.duration_since(self.sent) < REOFFER_INTERVAL {
            self.deferred = true;
            return None;
        }

        Some(self.send(now))
    }

    /// Called when the pixel size changed. Resends the offer so encoding starts at the new size; without it the server
    /// stops encoding.
    pub fn resize(&mut self, now: Instant) -> Option<Vec<u8>> {
        self.answered.is_some().then(|| self.send(now))
    }

    fn send(&mut self, now: Instant) -> Vec<u8> {
        self.sent = now;
        self.offers += 1;
        self.retry = None;
        self.deferred = false;
        self.offer.encode()
    }
}

#[cfg(test)]
mod tests {
    use super::{Negotiation, REOFFER_INTERVAL, SILENT_ATTEMPTS, TIMEOUT};
    use crate::offer::Offer;
    use std::time::Instant;

    #[test]
    fn silent_server_is_given_up_after_a_few_offers() {
        let mut negotiation = Negotiation::new(Offer::new(false, crate::media::VideoCodec::H264));
        let mut now = Instant::now();
        negotiation.start(now);

        for _ in 1..SILENT_ATTEMPTS {
            now = negotiation.deadline().unwrap();
            assert!(negotiation.expire(now).is_some());
        }

        now = negotiation.deadline().unwrap();
        assert_eq!(now, negotiation.sent + TIMEOUT);
        assert!(negotiation.expire(now).is_none());
        assert_eq!(negotiation.offers(), SILENT_ATTEMPTS);
    }

    #[test]
    fn nothing_is_resent_before_an_answer_or_the_timeout() {
        let mut negotiation = Negotiation::new(Offer::new(false, crate::media::VideoCodec::H264));
        let now = Instant::now();
        negotiation.start(now);

        assert_eq!(negotiation.deadline(), Some(now + TIMEOUT));
        assert!(negotiation.reoffer(now + TIMEOUT).is_none());
    }

    #[test]
    fn reoffers_after_the_answer_are_spaced() {
        let mut negotiation = Negotiation::new(Offer::new(false, crate::media::VideoCodec::H264));
        let now = Instant::now();
        negotiation.start(now);
        negotiation.answered = Some(now);

        assert!(negotiation.reoffer(now).is_none());
        assert_eq!(negotiation.deadline(), Some(now + REOFFER_INTERVAL));
        assert!(negotiation.expire(now + REOFFER_INTERVAL).is_some());
        assert_eq!(negotiation.deadline(), None);
        assert!(negotiation.reoffer(now + REOFFER_INTERVAL).is_none());
    }
}
