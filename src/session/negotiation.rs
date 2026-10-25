use crate::offer::{Answer, Offer};
use std::time::{Duration, Instant};

// How long the server may take to ask for an offer and answer it before the start is given up. It asks about 1.5
// seconds after the connection opens.
const TIMEOUT: Duration = Duration::from_secs(12);

// Minimum interval between offers resent for keyframes after negotiation, so they do not overlap before the server
// opens a new stream. Requests within the interval are deferred.
const REOFFER_INTERVAL: Duration = Duration::from_secs(2);

/// Sends the media offer whenever the server asks for one, as Apple's client does: at the start, after a display
/// configuration, and after the Mac's display wakes. The server opens a new stream for each offer and answers with its
/// size. It returns the messages to send; the caller sends them.
///
/// - The start is given up when no answer with a size arrives within [`TIMEOUT`].
/// - To get a keyframe while the screen is still, the offer also goes out unasked, spaced by [`REOFFER_INTERVAL`]. The
///   server opens a new stream and sends a config and keyframe.
pub struct Negotiation {
    offer: Offer,
    started: Instant,
    sent: Instant,
    // Offers sent so far
    offers: u32,
    answered: Option<Instant>,
    // A keyframe resend deferred because it fell within the interval
    deferred: bool,
}

impl Negotiation {
    pub fn new(offer: Offer) -> Self {
        let now = Instant::now();

        Self {
            offer,
            started: now,
            sent: now,
            offers: 0,
            answered: None,
            deferred: false,
        }
    }

    /// The offer the server asked for
    pub fn request(&mut self, now: Instant) -> Vec<u8> {
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

    /// When the start is given up, or once negotiated, when the deferred resend goes out
    pub fn deadline(&self) -> Option<Instant> {
        match self.answered {
            None => Some(self.started + TIMEOUT),
            Some(_) => self.deferred.then_some(self.sent + REOFFER_INTERVAL),
        }
    }

    /// Looks for an answer in a body carried by a framebuffer update, returning `true` for the first answer with a
    /// size. An answer without one means the encoder is not ready, and the server asks again once it is.
    pub fn answer(&mut self, body: &[u8], now: Instant) -> bool {
        let first = Answer::find(body).is_some_and(|answer| answer.ready) && self.answered.is_none();

        if first {
            self.answered = Some(now);
        }

        first
    }

    /// Called once [`Self::deadline`] passes. Returns the deferred resend, or `None` when the start is given up.
    pub fn expire(&mut self, now: Instant) -> Option<Vec<u8>> {
        self.answered.is_some().then(|| self.send(now))
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

    fn send(&mut self, now: Instant) -> Vec<u8> {
        self.sent = now;
        self.offers += 1;
        self.deferred = false;
        self.offer.encode()
    }
}

#[cfg(test)]
mod tests {
    use super::{Negotiation, REOFFER_INTERVAL, TIMEOUT};
    use crate::offer::Offer;
    use std::time::Instant;

    #[test]
    fn a_server_that_never_answers_is_given_up() {
        let mut negotiation = Negotiation::new(Offer::new(false, crate::media::VideoCodec::H264));
        let deadline = negotiation.deadline().unwrap();

        assert_eq!(deadline, negotiation.started + TIMEOUT);
        assert!(negotiation.expire(deadline).is_none());
    }

    #[test]
    fn nothing_is_offered_unasked_before_an_answer() {
        let mut negotiation = Negotiation::new(Offer::new(false, crate::media::VideoCodec::H264));
        let now = Instant::now();
        negotiation.request(now);

        assert!(negotiation.reoffer(now + TIMEOUT).is_none());
        assert_eq!(negotiation.offers(), 1);
    }

    #[test]
    fn reoffers_after_the_answer_are_spaced() {
        let mut negotiation = Negotiation::new(Offer::new(false, crate::media::VideoCodec::H264));
        let now = Instant::now();
        negotiation.request(now);
        negotiation.answered = Some(now);

        assert!(negotiation.reoffer(now).is_none());
        assert_eq!(negotiation.deadline(), Some(now + REOFFER_INTERVAL));
        assert!(negotiation.expire(now + REOFFER_INTERVAL).is_some());
        assert_eq!(negotiation.deadline(), None);
        assert!(negotiation.reoffer(now + REOFFER_INTERVAL).is_none());
    }
}
