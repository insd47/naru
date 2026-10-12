//! RTP payload of the server's audio.
//!
//! - AAC-ELD + SBR, 48 kHz stereo. Each RTP packet carries one access unit without an AU header (480 frames per packet).
//! - Silence is sent as four zero bytes, which can't be decoded, so they are skipped.
use super::RtpPacket;

const PAYLOAD_TYPE: u8 = 101;
const SILENCE: [u8; 4] = [0; 4];

/// One audio access unit.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AudioFrame {
    /// RTP timestamp (48 kHz)
    pub timestamp: u32,
    /// AAC-ELD access unit
    pub data: Vec<u8>,
}

impl AudioFrame {
    /// Decoder config (AudioSpecificConfig): object type 39 (ELD), 48 kHz, stereo, 480 frames, SBR on
    pub const CONFIG: [u8; 6] = [0xf8, 0xe6, 0x51, 0x32, 0xe0, 0x00];

    /// Returns `None` for non-audio packets and silence.
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
