//! 서버 오디오의 RTP 페이로드입니다.
//!
//! - AAC-ELD + SBR, 48 kHz 스테레오이며, RTP 패킷 하나에 접근 단위 하나가 AU 머리 없이 실립니다(패킷당 480 프레임).
//! - 소리가 없으면 0으로 된 4바이트를 보냅니다. 디코딩할 수 없는 값이라 건너뜁니다.
use super::RtpPacket;

const PAYLOAD_TYPE: u8 = 101;
const SILENCE: [u8; 4] = [0; 4];

/// 오디오 접근 단위 하나입니다.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AudioFrame {
    /// RTP 타임스탬프(48 kHz)
    pub timestamp: u32,
    /// AAC-ELD 접근 단위
    pub data: Vec<u8>,
}

impl AudioFrame {
    /// 디코더 설정(AudioSpecificConfig): 객체 유형 39(ELD), 48 kHz, 스테레오, 480 프레임, SBR 켬
    pub const CONFIG: [u8; 6] = [0xf8, 0xe6, 0x51, 0x32, 0xe0, 0x00];

    /// 오디오 패킷이 아니거나 무음이면 `None`입니다.
    pub fn depacketize(packet: RtpPacket) -> Option<Self> {
        let audio = packet.payload_type == PAYLOAD_TYPE && !packet.payload.is_empty() && packet.payload != SILENCE;

        audio.then_some(Self {
            timestamp: packet.timestamp,
            data: packet.payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::AudioFrame;
    use crate::media::RtpPacket;

    fn packet(payload_type: u8, payload: &[u8]) -> RtpPacket {
        RtpPacket {
            marker: false,
            payload_type,
            sequence: 1,
            timestamp: 480,
            ssrc: 7,
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn audio_payload_is_one_access_unit() {
        let frame = AudioFrame::depacketize(packet(101, &[0x21, 0x1b, 0x80])).unwrap();
        assert_eq!((frame.timestamp, frame.data.as_slice()), (480, &[0x21, 0x1b, 0x80][..]));
    }

    #[test]
    fn silence_and_other_payload_types_are_skipped() {
        assert_eq!(AudioFrame::depacketize(packet(101, &[0, 0, 0, 0])), None);
        assert_eq!(AudioFrame::depacketize(packet(100, &[1, 2, 3])), None);
    }
}
