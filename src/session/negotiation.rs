use crate::offer::{Answer, Offer};
use std::time::{Duration, Instant};

// 응답에 크기가 없으면 서버의 인코더가 아직 준비되지 않은 것이므로 같은 오퍼를 다시 보냅니다.
// 응답이 오기 전에는 다시 보내지 않습니다. 서버는 오퍼마다 스트림을 만드므로, 성급히 보내면 스트림이 여럿 생겨 시작이 끊깁니다.
const RETRY: Duration = Duration::from_millis(200);
const ATTEMPTS: u32 = 16;

// 응답이 아예 없을 때 같은 오퍼를 다시 보내기까지 기다리는 시간과, 그렇게 보내는 횟수입니다. 실측한 응답 시간은 1.5초 안팎입니다.
const TIMEOUT: Duration = Duration::from_secs(4);
const SILENT_ATTEMPTS: u32 = 3;

// 협상이 끝난 뒤 키프레임을 위해 오퍼를 다시 보내는 최소 간격입니다. 서버가 새 스트림을 열기 전에 겹쳐 보내지 않게 하고, 간격 안의 요청은 미뤘다 보냅니다.
const REOFFER_INTERVAL: Duration = Duration::from_secs(2);

/// 미디어 오퍼를 보내고 크기가 있는 응답을 기다립니다. 보낼 메시지를 돌려주고, 보내는 일은 호출자가 합니다.
///
/// - 크기 없는 응답이 오면 잠시 뒤 다시 보내고, 응답이 아예 없으면 [`TIMEOUT`]마다 다시 보냅니다. 횟수를 넘으면 포기합니다.
/// - 응답 없이 시간이 지난 횟수는 응답이 오면 처음부터 셉니다.
/// - 협상이 끝난 뒤에도 오퍼를 다시 보낼 때가 있습니다(픽셀 크기 변경, 화면이 멈춘 동안의 키프레임). 서버는 새 스트림을 열며 설정과 키프레임을 보냅니다.
/// - 키프레임을 위한 재전송은 간격을 둡니다. 크기 변경은 늘 보냅니다.
pub struct Negotiation {
    offer: Offer,
    sent: Instant,
    // 지금까지 보낸 오퍼의 수와, 마지막 응답 뒤로 응답 없이 시간이 지난 수
    offers: u32,
    silent: u32,
    // 크기 없는 응답을 받아 다시 보내기로 한 시각
    retry: Option<Instant>,
    answered: Option<Instant>,
    // 간격 안에 들어와 미뤄 둔 키프레임용 재전송
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

    /// 첫 오퍼
    pub fn start(&mut self, now: Instant) -> Vec<u8> {
        self.send(now)
    }

    /// 크기가 있는 응답을 받은 시각. 아직 협상 중이면 `None`입니다.
    pub fn answered(&self) -> Option<Instant> {
        self.answered
    }

    /// 지금까지 보낸 오퍼의 수. 협상 뒤의 재전송도 셉니다.
    pub fn offers(&self) -> u32 {
        self.offers
    }

    /// 다시 보낼 시각. 협상이 끝났고 미뤄 둔 재전송도 없으면 `None`입니다.
    pub fn deadline(&self) -> Option<Instant> {
        match self.answered {
            None => Some(self.retry.unwrap_or(self.sent + TIMEOUT)),
            Some(_) => self.deferred.then_some(self.sent + REOFFER_INTERVAL),
        }
    }

    /// 프레임버퍼 갱신에 실린 본문에서 응답을 찾습니다. 크기가 있는 첫 응답이면 `true`입니다.
    pub fn answer(&mut self, body: &[u8], now: Instant) -> bool {
        let Some(answer) = Answer::find(body).filter(|_| self.answered.is_none()) else {
            return false;
        };

        self.silent = 0;

        if answer.size.is_some() {
            self.answered = Some(now);
            return true;
        }

        self.retry.get_or_insert(now + RETRY);
        false
    }

    /// [`Self::deadline`]이 지났습니다. 다시 보낼 오퍼를 돌려주고, 협상 중에 횟수를 넘었으면 `None`입니다.
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

    /// 키프레임을 받으려고 오퍼를 다시 보냅니다. 아직 협상 중이면 `None`이고, 방금 보냈으면 미뤘다가 [`Self::deadline`]에 보냅니다.
    pub fn reoffer(&mut self, now: Instant) -> Option<Vec<u8>> {
        self.answered?;

        if now.duration_since(self.sent) < REOFFER_INTERVAL {
            self.deferred = true;
            return None;
        }

        Some(self.send(now))
    }

    /// 픽셀 크기가 바뀌었습니다. 새 크기로 인코딩을 시작하게 오퍼를 다시 보냅니다. 보내지 않으면 서버가 인코딩을 멈춥니다.
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
        let mut negotiation = Negotiation::new(Offer::new(false, crate::Codec::H264));
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
        let mut negotiation = Negotiation::new(Offer::new(false, crate::Codec::H264));
        let now = Instant::now();
        negotiation.start(now);

        assert_eq!(negotiation.deadline(), Some(now + TIMEOUT));
        assert!(negotiation.reoffer(now + TIMEOUT).is_none());
    }

    #[test]
    fn reoffers_after_the_answer_are_spaced() {
        let mut negotiation = Negotiation::new(Offer::new(false, crate::Codec::H264));
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
