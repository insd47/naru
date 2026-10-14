//! SRTP and SRTCP (RFC 3711), with AES-256 counter mode encryption and 80-bit HMAC-SHA1 authentication.
use super::RtpPacket;
use crate::offer::KEY_BLOB_BYTES;
use aes::cipher::{KeyIvInit, StreamCipher};
use aes::{Aes128, Aes256};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use std::collections::HashMap;

const TAG_BYTES: usize = 10;
const MASTER_KEY_BYTES: usize = 32;

// Key derivation labels (RFC 3711 4.3.1). SRTCP uses these plus 3.
const LABEL_CIPHER: u8 = 0;
const LABEL_AUTH: u8 = 1;
const LABEL_SALT: u8 = 2;
const LABEL_RTCP: u8 = 3;

type Ctr<C> = ctr::Ctr128BE<C>;

/// SRTP and SRTCP cipher state built from one master key blob.
///
/// - The rollover counter (ROC) of received RTP is tracked per SSRC. A shared one stalls the stream after 65,536
///   packets.
pub struct SrtpContext {
    rtp: SessionKeys,
    rtcp: SessionKeys,
    received: HashMap<u32, Rollover>,
    sent: HashMap<u32, Rollover>,
    rtcp_index: u32,
}

struct SessionKeys {
    cipher: [u8; 32],
    auth: [u8; 20],
    salt: [u8; 14],
}

// Highest sequence number seen so far on one SSRC, and the ROC at that point.
#[derive(Clone, Copy)]
struct Rollover {
    roc: u32,
    highest: u16,
}

impl SrtpContext {
    pub fn new(blob: &[u8; KEY_BLOB_BYTES]) -> Self {
        let (key, salt) = blob.split_at(MASTER_KEY_BYTES);
        let salt: &[u8; 14] = salt.try_into().expect("the blob is 32 + 14 bytes");

        Self {
            rtp: SessionKeys::derive(key, salt, 0),
            rtcp: SessionKeys::derive(key, salt, LABEL_RTCP),
            received: HashMap::new(),
            sent: HashMap::new(),
            rtcp_index: 0,
        }
    }

    /// Authenticates and decrypts a received SRTP packet into a plaintext RTP packet. Returns `None` on authentication
    /// failure, and the packet should be dropped.
    pub fn unprotect_rtp(&mut self, packet: &[u8]) -> Option<Vec<u8>> {
        let header_len = RtpPacket::header_len(packet)?;
        let body_len = packet.len().checked_sub(TAG_BYTES).filter(|&len| len >= header_len)?;
        let (body, tag) = packet.split_at(body_len);

        let ssrc = u32::from_be_bytes(packet[8..12].try_into().ok()?);
        let sequence = u16::from_be_bytes([packet[2], packet[3]]);
        let state = self.received.get(&ssrc).copied();
        let roc = Rollover::estimate(state, sequence);

        let mut mac = self.rtp.mac();
        mac.update(body);
        mac.update(&roc.to_be_bytes());
        mac.verify_truncated_left(tag).ok()?;

        let mut plain = body.to_vec();
        let index = (u64::from(roc) << 16) | u64::from(sequence);
        self.rtp.apply(ssrc, index, &mut plain[header_len..]);

        self.received.insert(ssrc, Rollover::advance(state, roc, sequence));
        Some(plain)
    }

    /// Encrypts a plaintext RTP packet with SRTP.
    pub fn protect_rtp(&mut self, packet: &[u8]) -> Option<Vec<u8>> {
        let header_len = RtpPacket::header_len(packet)?;
        let ssrc = u32::from_be_bytes(packet[8..12].try_into().ok()?);
        let sequence = u16::from_be_bytes([packet[2], packet[3]]);
        let state = self.sent.get(&ssrc).copied();
        let roc = Rollover::estimate(state, sequence);

        let mut protected = packet.to_vec();
        let index = (u64::from(roc) << 16) | u64::from(sequence);
        self.rtp.apply(ssrc, index, &mut protected[header_len..]);

        let mut mac = self.rtp.mac();
        mac.update(&protected);
        mac.update(&roc.to_be_bytes());
        protected.extend_from_slice(&mac.finalize().into_bytes()[..TAG_BYTES]);

        self.sent.insert(ssrc, Rollover::advance(state, roc, sequence));
        Some(protected)
    }

    /// Encrypts an RTCP compound packet with SRTCP. The first 8 bytes (the first packet's header and sender SSRC) stay
    /// in plaintext.
    pub fn protect_rtcp(&mut self, compound: &[u8]) -> Option<Vec<u8>> {
        let ssrc = u32::from_be_bytes(compound.get(4..8)?.try_into().ok()?);
        let index = self.rtcp_index;
        self.rtcp_index = (self.rtcp_index + 1) & 0x7fff_ffff;

        let mut protected = compound.to_vec();
        self.rtcp.apply(ssrc, u64::from(index), &mut protected[8..]);
        protected.extend_from_slice(&(index | 1 << 31).to_be_bytes());

        let mut mac = self.rtcp.mac();
        mac.update(&protected);
        protected.extend_from_slice(&mac.finalize().into_bytes()[..TAG_BYTES]);
        Some(protected)
    }

    /// Highest extended sequence number received on this SSRC (`ROC × 65536 + sequence number`), for receiver reports
    pub fn highest_received(&self, ssrc: u32) -> Option<u32> {
        let state = self.received.get(&ssrc)?;
        Some(state.roc << 16 | u32::from(state.highest))
    }
}

impl SessionKeys {
    // Derives session keys from the master key and salt, with a key derivation rate of 0.
    fn derive(key: &[u8], salt: &[u8; 14], base: u8) -> Self {
        let mut keys = Self {
            cipher: [0; 32],
            auth: [0; 20],
            salt: [0; 14],
        };

        prf(key, salt, base + LABEL_CIPHER, &mut keys.cipher);
        prf(key, salt, base + LABEL_AUTH, &mut keys.auth);
        prf(key, salt, base + LABEL_SALT, &mut keys.salt);
        keys
    }

    // IV = (salt · 2^16) XOR (SSRC · 2^64) XOR (index · 2^16)
    fn apply(&self, ssrc: u32, index: u64, data: &mut [u8]) {
        let mut iv = [0; 16];
        iv[..14].copy_from_slice(&self.salt);

        for (byte, value) in iv[4..8].iter_mut().zip(ssrc.to_be_bytes()) {
            *byte ^= value;
        }

        for (byte, value) in iv[8..14].iter_mut().zip(&index.to_be_bytes()[2..]) {
            *byte ^= value;
        }

        Ctr::<Aes256>::new(&self.cipher.into(), &iv.into()).apply_keystream(data);
    }

    fn mac(&self) -> Hmac<Sha1> {
        Hmac::new_from_slice(&self.auth).expect("HMAC accepts any key length")
    }
}

impl Rollover {
    // Estimates which ROC a received sequence number belongs to (RFC 3711 Appendix A).
    fn estimate(state: Option<Self>, sequence: u16) -> u32 {
        let Some(Self { roc, highest }) = state else {
            return 0;
        };

        if highest < 0x8000 {
            if sequence > highest && sequence - highest > 0x8000 {
                return roc.wrapping_sub(1);
            }
        } else if sequence < highest - 0x8000 {
            return roc.wrapping_add(1);
        }

        roc
    }

    fn advance(state: Option<Self>, roc: u32, sequence: u16) -> Self {
        let next = Self { roc, highest: sequence };

        match state {
            Some(current) if (roc, sequence) <= (current.roc, current.highest) => current,
            _ => next,
        }
    }
}

// Derives one session key with the AES counter mode PRF: x = salt XOR (label · 2^48), and the IV is x · 2^16.
// The master key is 32 bytes for AES-256; 16-byte AES-128 keys are also accepted for the key derivation test vectors.
fn prf(key: &[u8], salt: &[u8; 14], label: u8, output: &mut [u8]) {
    let mut iv = [0; 16];
    iv[..14].copy_from_slice(salt);
    iv[7] ^= label;
    output.fill(0);

    match key.len() {
        16 => Ctr::<Aes128>::new_from_slices(key, &iv)
            .expect("length was checked")
            .apply_keystream(output),
        _ => Ctr::<Aes256>::new_from_slices(key, &iv)
            .expect("the master key is 32 bytes")
            .apply_keystream(output),
    }
}

#[cfg(test)]
mod tests {
    use super::{Rollover, SrtpContext, prf};

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }

    fn blob() -> [u8; 46] {
        std::array::from_fn(|at| at as u8)
    }

    // Key derivation test vectors from RFC 3711 Appendix B.3 (AES-128).
    #[test]
    fn key_derivation_matches_rfc_3711_vectors() {
        let key = hex("E1F97A0D3E018BE0D64FA32C06DE4139");
        let salt: [u8; 14] = hex("0EC675AD498AFEEBB6960B3AABE6").try_into().unwrap();

        let mut cipher = [0; 16];
        let mut auth = [0; 20];
        let mut session_salt = [0; 14];
        prf(&key, &salt, 0, &mut cipher);
        prf(&key, &salt, 1, &mut auth);
        prf(&key, &salt, 2, &mut session_salt);

        assert_eq!(cipher.to_vec(), hex("C61E7A93744F39EE10734AFE3FF7A087"));
        assert_eq!(session_salt.to_vec(), hex("30CBBC08863D8C85D49DB34A9AE1"));
        assert_eq!(auth.to_vec(), hex("CEBE321F6FF7716B6FD4AB49AF256A156D38BAA4"));
    }

    // The two packets below were computed independently with OpenSSL's AES-256 and Python's hmac. The master blob is 0,
    // 1, ..., 45.
    const RTP: &str =
        "8060123401020304deadbeef857bd59c5e9a1978315235b481e5749b10ebb1aea63b2b8200ddd2ba146775f2a80b858f6a983c8f13dd";
    const RTCP: &str = "80c9000111223344b73ebd2f6916ba139338494b800000058a5a24ca6e4543a3661f";

    #[test]
    fn rtp_packet_matches_an_independent_implementation() {
        let packet = hex(RTP);
        let plain = SrtpContext::new(&blob()).unprotect_rtp(&packet).unwrap();
        assert_eq!(&plain[12..], b"naru srtp test vector 0123456789");
        assert_eq!(SrtpContext::new(&blob()).protect_rtp(&plain).unwrap(), packet);
    }

    #[test]
    fn rtcp_packet_matches_an_independent_implementation() {
        let mut context = SrtpContext::new(&blob());
        context.rtcp_index = 5;

        let compound = hex("80c900011122334481ce000211223344deadbeef");

        assert_eq!(context.protect_rtcp(&compound).unwrap(), hex(RTCP));
    }

    #[test]
    fn tampered_packet_is_dropped() {
        let mut packet = hex(RTP);
        packet[20] ^= 1;
        assert_eq!(SrtpContext::new(&blob()).unprotect_rtp(&packet), None);
    }

    #[test]
    fn rollover_counter_advances_across_the_sequence_wrap() {
        let mut sender = SrtpContext::new(&blob());
        let mut receiver = SrtpContext::new(&blob());

        for sequence in [65_534u16, 65_535, 0, 1, 65_535, 2] {
            let mut packet = vec![0x80, 0x60, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0xaa];
            packet[2..4].copy_from_slice(&sequence.to_be_bytes());

            let protected = sender.protect_rtp(&packet).unwrap();
            assert_eq!(
                receiver.unprotect_rtp(&protected).unwrap(),
                packet,
                "sequence {sequence}"
            );
        }

        assert_eq!(receiver.highest_received(7), Some(1 << 16 | 2));
    }

    #[test]
    fn rollover_estimate_follows_rfc_3711_appendix_a() {
        let state = |roc, highest| Some(Rollover { roc, highest });

        assert_eq!(Rollover::estimate(None, 9), 0);
        assert_eq!(Rollover::estimate(state(3, 65_000), 10), 4);
        assert_eq!(Rollover::estimate(state(3, 10), 65_000), 2);
        assert_eq!(Rollover::estimate(state(3, 40_000), 30_000), 3);
    }
}
