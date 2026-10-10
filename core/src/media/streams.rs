use super::RtpPacket;
use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;
use std::time::Instant;

// 새 SSRC로 넘어가는 조건입니다. 현재 스트림에서 이 시간 동안 프레임이 나오지 않았고, 새 SSRC에서 패킷이 이만큼 왔어야 합니다.
const SWITCH_AFTER: Duration = Duration::from_secs(2);
const MIN_PACKETS: usize = 5;

// 후보 SSRC마다 붙잡아 두는 패킷 수입니다. 넘어갈 때 설정 패킷을 잃지 않도록, 고른 SSRC의 앞부분을 조립기에 다시 넣습니다.
const MAX_HELD: usize = 256;

/// 받은 비디오 패킷 가운데 디코딩할 SSRC를 고릅니다.
///
/// - 타일 1개를 요청하므로 한 묶음은 SSRC 하나입니다. 해상도 변경이나 세션 전환 뒤에는 서버가 새 SSRC로 보냅니다.
/// - 한 번 버린 SSRC로는 돌아가지 않습니다. 서버가 두 스트림을 잠시 함께 보낼 때 왔다 갔다 하는 것을 막습니다.
#[derive(Default)]
pub struct Streams {
    current: Option<u32>,
    abandoned: HashSet<u32>,
    held: HashMap<u32, Vec<RtpPacket>>,
    last_frame: Option<Instant>,
}

/// 패킷을 어디로 보낼지 정한 결과입니다.
#[derive(Debug, PartialEq)]
pub enum Routed {
    /// 현재 스트림의 패킷
    Current(RtpPacket),
    /// 이 SSRC로 넘어갔습니다. 새 조립기를 만들고 `packets`를 순서대로 넣습니다.
    Switched { ssrc: u32, packets: Vec<RtpPacket> },
    /// 후보 SSRC의 패킷을 붙잡아 두었거나 버렸습니다.
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

    /// 현재 스트림에서 프레임이 나왔음을 알립니다.
    pub fn mark_frame(&mut self, now: Instant) {
        self.last_frame = Some(now);
    }

    pub fn current(&self) -> Option<u32> {
        self.current
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
