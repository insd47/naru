//! 클라이언트가 보내는 RTCP 패킷입니다. 모두 복합 패킷으로 묶어 SRTCP로 암호화한 뒤 보냅니다.
//!
//! - 송신자 SSRC는 오퍼에서 협상한 클라이언트의 비디오 SSRC입니다. 다른 SSRC로 보내면 서버가 약 30초 뒤 스트림을 끊습니다.
//! - 손실률, 지터, LSR, DLSR은 0으로 보냅니다. 서버는 0이어도 세션을 유지합니다.

const RECEIVER_REPORT: u8 = 201;
const SENDER_REPORT: u8 = 200;
const FEEDBACK_TRANSPORT: u8 = 205;
const FEEDBACK_PAYLOAD: u8 = 206;
const LEGACY_FIR: u8 = 192;

const FORMAT_NACK: u8 = 1;
const FORMAT_PLI: u8 = 1;
const FORMAT_FIR: u8 = 4;

/// 수신 보고(RR)를 만듭니다. 보고 블록은 `(SSRC, 확장 순번)`마다 하나이며 최대 31개입니다.
pub fn receiver_report(sender: u32, sources: &[(u32, u32)]) -> Vec<u8> {
    let sources = &sources[..sources.len().min(31)];
    let mut packet = header(sources.len() as u8, RECEIVER_REPORT, 1 + 6 * sources.len());
    packet.extend_from_slice(&sender.to_be_bytes());

    for (ssrc, highest) in sources {
        packet.extend_from_slice(&ssrc.to_be_bytes());
        packet.extend_from_slice(&[0; 4]);
        packet.extend_from_slice(&highest.to_be_bytes());
        packet.extend_from_slice(&[0; 12]);
    }

    packet
}

/// 빈 송신 보고(SR)를 만듭니다. 서버의 생존 확인을 통과하도록 5초마다 수신 보고 앞에 붙입니다.
pub fn sender_report(sender: u32) -> Vec<u8> {
    let mut packet = header(0, SENDER_REPORT, 6);
    packet.extend_from_slice(&sender.to_be_bytes());
    packet.extend_from_slice(&[0; 20]);
    packet
}

/// 키프레임 요청 묶음을 만듭니다.
///
/// - 서버가 AVPF 방식(RFC 5104 FIR, RFC 4585 PLI)을 자주 무시하고 구식 FIR(RFC 2032)에 응답하므로 세 가지를 함께 보냅니다.
/// - `sequence`는 FIR 명령 순번이며 요청마다 1씩 늘립니다.
pub fn keyframe_request(sender: u32, target: u32, sequence: u8) -> Vec<u8> {
    let mut packet = receiver_report(sender, &[]);

    packet.extend(header(FORMAT_FIR, FEEDBACK_PAYLOAD, 4));
    packet.extend_from_slice(&sender.to_be_bytes());
    packet.extend_from_slice(&[0; 4]);
    packet.extend_from_slice(&target.to_be_bytes());
    packet.extend_from_slice(&[sequence, 0, 0, 0]);

    packet.extend(header(FORMAT_PLI, FEEDBACK_PAYLOAD, 2));
    packet.extend_from_slice(&sender.to_be_bytes());
    packet.extend_from_slice(&target.to_be_bytes());

    packet.extend(header(0, LEGACY_FIR, 1));
    packet.extend_from_slice(&target.to_be_bytes());
    packet
}

/// 재전송 요청(RFC 4585 Generic NACK)을 만듭니다. 연속한 손실은 기준 순번과 뒤 16개의 비트마스크로 묶습니다.
pub fn nack(sender: u32, target: u32, lost: &[u16]) -> Vec<u8> {
    let mut entries: Vec<(u16, u16)> = Vec::new();

    for &sequence in lost {
        let covered = entries.last_mut().and_then(|(base, mask)| {
            let offset = sequence.wrapping_sub(*base);
            (1..=16).contains(&offset).then(|| *mask |= 1 << (offset - 1))
        });

        if covered.is_none() {
            entries.push((sequence, 0));
        }
    }

    let mut packet = receiver_report(sender, &[]);
    packet.extend(header(FORMAT_NACK, FEEDBACK_TRANSPORT, 2 + entries.len()));
    packet.extend_from_slice(&sender.to_be_bytes());
    packet.extend_from_slice(&target.to_be_bytes());

    for (base, mask) in entries {
        packet.extend_from_slice(&base.to_be_bytes());
        packet.extend_from_slice(&mask.to_be_bytes());
    }

    packet
}

// 버전 2, 채움 없음. 길이는 머리를 뺀 32비트 단어 수입니다.
fn header(count: u8, kind: u8, words: usize) -> Vec<u8> {
    let mut header = vec![0x80 | count & 0x1f, kind];
    header.extend_from_slice(&(words as u16).to_be_bytes());
    header
}

#[cfg(test)]
mod tests {
    use super::{keyframe_request, nack, receiver_report, sender_report};

    // 복합 패킷을 (유형, 형식, 길이)로 나눕니다. 각 패킷의 길이 필드가 실제 길이와 맞아야 끝까지 나뉩니다.
    fn split(mut compound: &[u8]) -> Vec<(u8, u8, usize)> {
        let mut packets = Vec::new();

        while !compound.is_empty() {
            let len = (u16::from_be_bytes([compound[2], compound[3]]) as usize + 1) * 4;
            packets.push((compound[1], compound[0] & 0x1f, len));
            compound = &compound[len..];
        }

        packets
    }

    #[test]
    fn report_block_carries_the_extended_highest_sequence() {
        let packet = receiver_report(1, &[(0xabcd, 2 << 16 | 7)]);
        assert_eq!(split(&packet), [(201, 1, 32)]);
        assert_eq!(&packet[8..12], 0xabcdu32.to_be_bytes());
        assert_eq!(&packet[16..20], (2u32 << 16 | 7).to_be_bytes());
    }

    #[test]
    fn empty_sender_report_is_28_bytes() {
        assert_eq!(split(&sender_report(1)), [(200, 0, 28)]);
    }

    #[test]
    fn keyframe_request_bundles_rr_fir_pli_and_legacy_fir() {
        let packet = keyframe_request(1, 2, 9);
        assert_eq!(split(&packet), [(201, 0, 8), (206, 4, 20), (206, 1, 12), (192, 0, 8)]);
        assert_eq!(&packet[8 + 12..8 + 17], [0, 0, 0, 2, 9]);
        assert_eq!(&packet[packet.len() - 8..], [0x80, 192, 0, 1, 0, 0, 0, 2]);
    }

    #[test]
    fn consecutive_losses_share_one_bitmask_entry() {
        // 65535와 0은 65534 뒤 1, 2번째라 같은 항목에 들어가고, 15와 40은 16칸 밖이라 새 항목이 됩니다.
        let packet = nack(1, 2, &[65_534, 65_535, 0, 15, 40]);
        assert_eq!(split(&packet), [(201, 0, 8), (205, 1, 24)]);
        assert_eq!(&packet[20..], [0xff, 0xfe, 0, 3, 0, 15, 0, 0, 0, 40, 0, 0]);
    }
}
