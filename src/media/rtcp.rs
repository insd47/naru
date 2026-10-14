//! RTCP packets the client sends. All of them go out encrypted with SRTCP, the reports and requests as compound packets.
//!
//! - The sender SSRC is the client video SSRC negotiated in the offer. With any other SSRC, the server cuts the stream after about 30 seconds.
//! - Loss, jitter, LSR, and DLSR are sent as 0; the server keeps the session alive regardless.
//! - No retransmission requests (NACK): the server answers none, to Apple's client or to this one, and Apple's client sends none.

const RECEIVER_REPORT: u8 = 201;
const SENDER_REPORT: u8 = 200;
const FEEDBACK_PAYLOAD: u8 = 206;
const LEGACY_FIR: u8 = 192;
const APPLICATION: u8 = 204;

const LTR_ACKNOWLEDGMENT: u32 = 5;

const FORMAT_PLI: u8 = 1;
const FORMAT_FIR: u8 = 4;

/// Builds a receiver report (RR) with one report block per `(SSRC, extended sequence number)`, up to 31.
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

/// Builds an empty sender report (SR). It goes in front of the receiver report every 5 seconds to pass the server's liveness check.
pub fn sender_report(sender: u32) -> Vec<u8> {
    let mut packet = header(0, SENDER_REPORT, 6);
    packet.extend_from_slice(&sender.to_be_bytes());
    packet.extend_from_slice(&[0; 20]);
    packet
}

/// Builds a bundle of keyframe requests.
///
/// - The server often ignores the AVPF requests (RFC 5104 FIR, RFC 4585 PLI) and answers the legacy FIR (RFC 2032), so all three are sent.
/// - `sequence` is the FIR command sequence number, incremented by 1 per request.
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

/// Builds an LTR acknowledgment: an APP packet of subtype 5 naming the last top-band picture received intact by its DONL.
///
/// - After a loss the server references the acknowledged picture instead of sending a keyframe. It silently ignores any
///   other counter in place of the DONL.
/// - Unlike the other packets here, it goes to the server's video port.
pub fn ltr_acknowledgment(sender: u32, donl: u16) -> Vec<u8> {
    let mut packet = header(0, APPLICATION, 3);
    packet.extend_from_slice(&sender.to_be_bytes());
    packet.extend_from_slice(&LTR_ACKNOWLEDGMENT.to_be_bytes());
    packet.extend_from_slice(&u32::from(donl).to_be_bytes());
    packet
}

// Version 2, no padding. The length is the number of 32-bit words minus one.
fn header(count: u8, kind: u8, words: usize) -> Vec<u8> {
    let mut header = vec![0x80 | count & 0x1f, kind];
    header.extend_from_slice(&(words as u16).to_be_bytes());
    header
}

#[cfg(test)]
mod tests {
    use super::{keyframe_request, ltr_acknowledgment, receiver_report, sender_report};

    // Splits a compound packet into (type, format, length). The split reaches the end only if every length field is correct.
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
    fn ltr_acknowledgment_is_an_app_packet_carrying_the_don() {
        let packet = ltr_acknowledgment(0x0102_0304, 0xbeef);
        assert_eq!(split(&packet), [(204, 0, 16)]);
        assert_eq!(packet, [0x80, 0xcc, 0, 3, 1, 2, 3, 4, 0, 0, 0, 5, 0, 0, 0xbe, 0xef]);
    }
}
