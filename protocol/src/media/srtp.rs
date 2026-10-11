//! SRTP와 SRTCP(RFC 3711)입니다. 암호는 AES-256 카운터 모드, 인증은 HMAC-SHA1 80비트입니다.
use super::RtpPacket;
use crate::offer::KEY_BLOB_BYTES;
use aes::Aes128;
use aes::Aes256;
use aes::cipher::KeyIvInit;
use aes::cipher::StreamCipher;
use hmac::Hmac;
use hmac::Mac;
use sha1::Sha1;
use std::collections::HashMap;

const TAG_BYTES: usize = 10;
const MASTER_KEY_BYTES: usize = 32;

// 키 유도 레이블입니다(RFC 3711 4.3.1). SRTCP는 3을 더한 레이블을 씁니다.
const LABEL_CIPHER: u8 = 0;
const LABEL_AUTH: u8 = 1;
const LABEL_SALT: u8 = 2;
const LABEL_RTCP: u8 = 3;

type Ctr<C> = ctr::Ctr128BE<C>;

/// 마스터 키 블롭 하나로 만든 SRTP와 SRTCP 암호 상태입니다.
///
/// - 수신한 RTP의 롤오버 카운터(ROC)는 SSRC마다 따로 추적합니다. 공유하면 6만 5천 패킷 뒤에 스트림이 멈춥니다.
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

// 한 SSRC에서 지금까지 본 가장 큰 순번과 그때의 ROC입니다.
#[derive(Clone, Copy)]
struct Rollover {
    roc: u32,
    highest: u16,
}

impl SrtpContext {
    pub fn new(blob: &[u8; KEY_BLOB_BYTES]) -> Self {
        let (key, salt) = blob.split_at(MASTER_KEY_BYTES);
        let salt: &[u8; 14] = salt.try_into().expect("블롭은 32 + 14바이트다");

        Self {
            rtp: SessionKeys::derive(key, salt, 0),
            rtcp: SessionKeys::derive(key, salt, LABEL_RTCP),
            received: HashMap::new(),
            sent: HashMap::new(),
            rtcp_index: 0,
        }
    }

    /// 받은 SRTP 패킷을 인증하고 풀어, 평문 RTP 패킷을 반환합니다. 인증에 실패하면 `None`이며 패킷을 버립니다.
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

    /// 평문 RTP 패킷을 SRTP로 암호화합니다.
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

    /// RTCP 복합 패킷을 SRTCP로 암호화합니다. 앞 8바이트(첫 패킷의 머리와 송신자 SSRC)는 평문으로 남습니다.
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

    /// 받은 SRTCP 패킷을 인증하고 풀어 RTCP 복합 패킷을 반환합니다.
    pub fn unprotect_rtcp(&mut self, packet: &[u8]) -> Option<Vec<u8>> {
        let body_len = packet.len().checked_sub(TAG_BYTES).filter(|&len| len >= 12)?;
        let (body, tag) = packet.split_at(body_len);

        let mut mac = self.rtcp.mac();
        mac.update(body);
        mac.verify_truncated_left(tag).ok()?;

        let (compound, trailer) = body.split_at(body_len - 4);
        let trailer = u32::from_be_bytes(trailer.try_into().ok()?);
        let ssrc = u32::from_be_bytes(compound[4..8].try_into().ok()?);
        let mut plain = compound.to_vec();

        if trailer & 1 << 31 != 0 {
            self.rtcp.apply(ssrc, u64::from(trailer & 0x7fff_ffff), &mut plain[8..]);
        }

        Some(plain)
    }

    /// 이 SSRC에서 받은 확장 순번의 최댓값(`ROC × 65536 + 순번`). 수신 보고에 씁니다.
    pub fn highest_received(&self, ssrc: u32) -> Option<u32> {
        let state = self.received.get(&ssrc)?;
        Some(state.roc << 16 | u32::from(state.highest))
    }
}

impl SessionKeys {
    // 마스터 키와 솔트로 세션 키를 유도합니다. 키 유도 비율은 0입니다.
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

    // IV = (솔트 · 2^16) XOR (SSRC · 2^64) XOR (인덱스 · 2^16)
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
        Hmac::new_from_slice(&self.auth).expect("HMAC은 모든 키 길이를 받는다")
    }
}

impl Rollover {
    // 받은 순번이 어느 ROC에 속하는지 추정합니다(RFC 3711 부록 A).
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

// AES 카운터 모드 PRF로 세션 키 하나를 만듭니다. x = 솔트 XOR (레이블 · 2^48)이며 IV는 x · 2^16입니다.
// 마스터 키는 AES-256용 32바이트이며, 키 유도 테스트 벡터를 위해 AES-128용 16바이트도 받습니다.
fn prf(key: &[u8], salt: &[u8; 14], label: u8, output: &mut [u8]) {
    let mut iv = [0; 16];
    iv[..14].copy_from_slice(salt);
    iv[7] ^= label;
    output.fill(0);

    match key.len() {
        16 => Ctr::<Aes128>::new_from_slices(key, &iv)
            .expect("길이를 확인했다")
            .apply_keystream(output),
        _ => Ctr::<Aes256>::new_from_slices(key, &iv)
            .expect("마스터 키는 32바이트다")
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

    // RFC 3711 부록 B.3의 키 유도 테스트 벡터(AES-128)입니다.
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

    // 아래 두 패킷은 openssl의 AES-256과 Python의 hmac으로 따로 계산한 기대값입니다. 마스터 블롭은 0, 1, …, 45입니다.
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
        assert_eq!(SrtpContext::new(&blob()).unprotect_rtcp(&hex(RTCP)).unwrap(), compound);
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
