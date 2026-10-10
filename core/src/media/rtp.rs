/// 복호화한 RTP 패킷입니다.
#[derive(Debug, Clone, PartialEq)]
pub struct RtpPacket {
    pub marker: bool,
    pub payload_type: u8,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    /// CSRC, 확장 머리, 채움을 뗀 페이로드
    pub payload: Vec<u8>,
}

impl RtpPacket {
    /// 평문 RTP 패킷을 해석합니다. 형식이 맞지 않으면 `None`입니다.
    pub fn parse(packet: &[u8]) -> Option<Self> {
        let header_len = Self::header_len(packet)?;
        let mut end = packet.len();

        // 채움 비트가 있으면 마지막 바이트가 채움의 길이입니다.
        if packet[0] & 0x20 != 0 {
            let padding = usize::from(*packet.last()?);
            end = end.checked_sub(padding).filter(|&end| end >= header_len)?;
        }

        Some(Self {
            marker: packet[1] & 0x80 != 0,
            payload_type: packet[1] & 0x7f,
            sequence: u16::from_be_bytes([packet[2], packet[3]]),
            timestamp: u32::from_be_bytes(packet[4..8].try_into().ok()?),
            ssrc: u32::from_be_bytes(packet[8..12].try_into().ok()?),
            payload: packet[header_len..end].to_vec(),
        })
    }

    /// CSRC와 확장 머리가 없는 패킷을 만듭니다.
    pub fn encode(&self) -> Vec<u8> {
        let mut packet = vec![0x80, u8::from(self.marker) << 7 | self.payload_type & 0x7f];
        packet.extend_from_slice(&self.sequence.to_be_bytes());
        packet.extend_from_slice(&self.timestamp.to_be_bytes());
        packet.extend_from_slice(&self.ssrc.to_be_bytes());
        packet.extend_from_slice(&self.payload);
        packet
    }

    /// 머리(고정 머리, CSRC, 확장 머리)의 길이. 버전이 2가 아니거나 잘렸으면 `None`입니다.
    pub fn header_len(packet: &[u8]) -> Option<usize> {
        let first = *packet.first()?;

        if first >> 6 != 2 || packet.len() < 12 {
            return None;
        }

        let mut len = 12 + 4 * usize::from(first & 0x0f);

        if first & 0x10 != 0 {
            let words = u16::from_be_bytes(packet.get(len + 2..len + 4)?.try_into().ok()?);
            len += 4 + 4 * usize::from(words);
        }

        (len <= packet.len()).then_some(len)
    }

    /// 같은 포트로 오는 RTP와 RTCP를 구분합니다(RFC 5761). 둘째 바이트의 하위 7비트가 64 ~ 95이면 RTCP입니다.
    pub fn is_rtcp(packet: &[u8]) -> bool {
        matches!(packet, [first, second, ..] if first >> 6 == 2 && (64..=95).contains(&(second & 0x7f)))
    }
}

#[cfg(test)]
mod tests {
    use super::RtpPacket;

    #[test]
    fn csrc_extension_and_padding_are_stripped() {
        let mut packet = vec![0xb1, 0xe0, 0, 5, 0, 0, 0, 9, 0, 0, 0, 7];
        packet.extend_from_slice(&[1, 1, 1, 1]);
        packet.extend_from_slice(&[0xbe, 0xde, 0, 1, 2, 2, 2, 2]);
        packet.extend_from_slice(b"nal");
        packet.extend_from_slice(&[0, 0, 3]);

        let parsed = RtpPacket::parse(&packet).unwrap();
        assert_eq!(parsed.payload, b"nal");
        assert!(parsed.marker);
        assert_eq!((parsed.payload_type, parsed.sequence, parsed.ssrc), (96, 5, 7));
    }

    #[test]
    fn extension_longer_than_the_packet_is_rejected() {
        let packet = [0x90, 0x60, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xbe, 0xde, 0xff, 0xff];
        assert_eq!(RtpPacket::parse(&packet), None);
    }

    #[test]
    fn rtcp_types_are_distinguished_from_rtp() {
        assert!(RtpPacket::is_rtcp(&[0x80, 200]));
        assert!(RtpPacket::is_rtcp(&[0x81, 206]));
        assert!(RtpPacket::is_rtcp(&[0x80, 192]));
        assert!(!RtpPacket::is_rtcp(&[0x80, 0x60]));
        assert!(!RtpPacket::is_rtcp(&[0x80, 0xe0]));
    }
}
