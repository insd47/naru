use crate::error::{Error, Result};
use crate::reader::Reader;
use num_bigint_dig::BigUint;
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::Digest;
use sha2::Sha256;
use sha2::Sha512;

// 그룹은 4096비트 MODP 소수이므로 모든 큰 수는 512바이트로 채워 해시합니다.
const PAD_BYTES: usize = 512;

// SRP가 아닌 경로로 빠진 서버는 이보다 짧은 본문을 보냅니다.
const MIN_CHALLENGE_BYTES: usize = 1000;

// 계정마다 다른 PBKDF2 반복 횟수입니다. 악의적인 서버가 클라이언트를 붙잡아 두지 못하도록 상한을 둡니다.
const MAX_ITERATIONS: u64 = 1_000_000;

// 증명 메시지는 길이 접두 뒤가 1076바이트로 고정되어 있으며, 머리 14바이트와 내용 뒤를 0으로 채웁니다.
const PROOF_BYTES: usize = 1076;
const PROOF_HEADER_BYTES: usize = 14;

/// 서버가 보낸 SRP 챌린지입니다.
pub struct Challenge {
    n: BigUint,
    g: BigUint,
    salt: Vec<u8>,
    b: BigUint,
    iterations: u32,
    // 서버가 고른 해시, 암호, KDF를 적은 문자열. 증명 메시지에 그대로 돌려보냅니다.
    options: Vec<u8>,
}

impl Challenge {
    /// 챌린지 본문(길이 접두를 뗀 것)을 해석합니다.
    ///
    /// - 본문이 짧으면 서버가 SRP가 아닌 경로를 고른 것이므로 `Unsupported`를 반환합니다.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < MIN_CHALLENGE_BYTES {
            return Err(Error::Unsupported);
        }

        // 머리 11바이트(`00 00 00 02`, 뒤따르는 길이 두 개, 제어 바이트)는 쓰지 않습니다.
        let mut reader = Reader::new(body, "SRP 챌린지");
        reader.skip(11)?;

        let len = reader.u16()? as usize;
        let n = number(reader.prefixed(len, PAD_BYTES + 1)?)?;
        let len = reader.u16()? as usize;
        let g = number(reader.prefixed(len, PAD_BYTES)?)?;
        let len = reader.u8()? as usize;
        let salt = reader.bytes(len)?.to_vec();
        let len = reader.u16()? as usize;
        let b = number(reader.prefixed(len, PAD_BYTES + 1)?)?;
        let iterations = reader.u64()?;
        let len = reader.u16()? as usize;
        let options = reader.prefixed(len, PROOF_BYTES - PROOF_HEADER_BYTES - 598)?.to_vec();

        if !(1..=MAX_ITERATIONS).contains(&iterations) {
            return Err(Error::Malformed("SRP 반복 횟수"));
        }

        // 공개값이 0과 합동이면 공유 비밀이 0으로 고정됩니다(RFC 5054 2.5.4).
        if n.bits() < 2 || &b % &n == BigUint::default() || &g % &n == BigUint::default() {
            return Err(Error::Malformed("SRP 챌린지"));
        }

        Ok(Self {
            n,
            g,
            salt,
            b,
            iterations: iterations as u32,
            options,
        })
    }

    /// 비밀번호로 증명을 계산합니다.
    ///
    /// - PBKDF2 반복 때문에 수백 밀리초가 걸릴 수 있습니다.
    pub fn respond(&self, password: &str) -> Proof {
        let mut a = [0; 64];
        OsRng.fill_bytes(&mut a);
        self.respond_with(password, &BigUint::from_bytes_be(&a))
    }

    fn respond_with(&self, password: &str, a: &BigUint) -> Proof {
        let Self { n, g, salt, b, .. } = self;
        let x = self.secret(password);

        let k = number_hash(&[&pad(n), &pad(g)]);
        let a_pub = g.modpow(a, n);
        let u = number_hash(&[&pad(&a_pub), &pad(b)]);

        // S = (B − k·g^x)^(a + u·x) mod N. 뺄셈이 음수가 되지 않도록 N을 더한 뒤 나눕니다.
        let kv = (&k * g.modpow(&x, n)) % n;
        let base = (b + n - kv) % n;
        let s = base.modpow(&(a + &u * &x), n);
        let key = hash(&[&pad(&s)]);

        let group = xor(&hash(&[&pad(n)]), &hash(&[&pad(g)]));
        let m1 = hash(&[&group, &hash(&[]), salt, &pad(&a_pub), &pad(b), &key]);
        let m2 = hash(&[&pad(&a_pub), &m1, &key]);

        let mut nonce = [0; 16];
        OsRng.fill_bytes(&mut nonce);

        let mut inner = Vec::new();
        inner.extend_from_slice(&(PAD_BYTES as u16).to_be_bytes());
        inner.extend_from_slice(&pad(&a_pub));
        inner.push(64);
        inner.extend_from_slice(&m1);
        inner.extend_from_slice(&(self.options.len() as u16).to_be_bytes());
        inner.extend_from_slice(&self.options);
        inner.push(16);
        inner.extend_from_slice(&nonce);

        let mut packet = Vec::with_capacity(PROOF_BYTES + 4);
        packet.extend_from_slice(&(PROOF_BYTES as u32).to_be_bytes());
        packet.extend_from_slice(&[1, 0, b'R', b'S', b'A', b'1', 0, 2]);
        packet.extend_from_slice(&(inner.len() as u16 + 4).to_be_bytes());
        packet.extend_from_slice(&[0, 0]);
        packet.extend_from_slice(&(inner.len() as u16).to_be_bytes());
        packet.extend_from_slice(&inner);
        packet.resize(PROOF_BYTES + 4, 0);

        let wrap_key = Sha256::digest(key)[..16].try_into().expect("SHA-256은 32바이트다");

        Proof {
            packet,
            wrap_key,
            server_proof: m2,
        }
    }

    // x = H(salt || H(":" || PBKDF2(비밀번호))). 사용자 이름 자리는 비어 있습니다.
    fn secret(&self, password: &str) -> BigUint {
        let mut dk = [0; 128];
        pbkdf2::pbkdf2_hmac::<Sha512>(password.as_bytes(), &self.salt, self.iterations, &mut dk);
        number_hash(&[&self.salt, &hash(&[b":", &dk])])
    }
}

/// 비밀번호 증명과, 인증에 성공한 뒤 쓸 키입니다.
pub struct Proof {
    /// 서버로 보낼 증명 메시지(1080바이트)
    pub packet: Vec<u8>,
    /// 첫 키 교체 메시지를 푸는 AES-128 키
    pub wrap_key: [u8; 16],
    server_proof: [u8; 64],
}

impl Proof {
    /// 서버가 인증 결과와 함께 보낸 블록에 기대한 서버 증명(M2)이 들어 있는지 확인합니다.
    ///
    /// - 블록 안에서 M2의 위치가 실측으로 확정되지 않았으므로, 블록 전체에서 찾습니다.
    pub fn verify(&self, block: &[u8]) -> bool {
        block
            .windows(self.server_proof.len())
            .any(|window| window == self.server_proof)
    }
}

// 앞의 0을 떼고 큰 수로 읽습니다. 512바이트보다 긴 값도 앞이 0이면 받아들입니다.
fn number(bytes: &[u8]) -> Result<BigUint> {
    let start = bytes.iter().position(|&byte| byte != 0).unwrap_or(bytes.len());

    if bytes.len() - start > PAD_BYTES {
        return Err(Error::Malformed("SRP 챌린지"));
    }

    Ok(BigUint::from_bytes_be(&bytes[start..]))
}

fn pad(value: &BigUint) -> Vec<u8> {
    let bytes = value.to_bytes_be();
    let mut padded = vec![0; PAD_BYTES.saturating_sub(bytes.len())];
    padded.extend_from_slice(&bytes);
    padded
}

fn hash(parts: &[&[u8]]) -> [u8; 64] {
    let mut digest = Sha512::new();

    for part in parts {
        digest.update(part);
    }

    digest.finalize().into()
}

fn number_hash(parts: &[&[u8]]) -> BigUint {
    BigUint::from_bytes_be(&hash(parts))
}

fn xor(left: &[u8; 64], right: &[u8; 64]) -> [u8; 64] {
    std::array::from_fn(|at| left[at] ^ right[at])
}

#[cfg(test)]
mod tests {
    use super::{Challenge, hash, number_hash, pad};
    use crate::error::Error;
    use num_bigint_dig::BigUint;
    use sha2::Digest;
    use sha2::Sha256;

    // 서버가 실제로 쓰는 값과 같은 모양의 옵션 문자열입니다.
    const OPTIONS: &[u8] = b"mda=SHA-512,replay_detection,conf+int=ChaCha20-Poly1305,kdf=SALTED-SHA512-PBKDF2";

    // 실제 서버의 챌린지와 같은 구성(머리 11바이트, 길이 접두 필드)으로 본문을 만듭니다.
    fn body(n: &BigUint, b: &BigUint, salt: &[u8], iterations: u64) -> Vec<u8> {
        let mut body = vec![0; 11];
        body.extend_from_slice(&512u16.to_be_bytes());
        body.extend_from_slice(&pad(n));
        body.extend_from_slice(&[0, 1, 5]);
        body.push(salt.len() as u8);
        body.extend_from_slice(salt);
        body.extend_from_slice(&512u16.to_be_bytes());
        body.extend_from_slice(&pad(b));
        body.extend_from_slice(&iterations.to_be_bytes());
        body.extend_from_slice(&(OPTIONS.len() as u16).to_be_bytes());
        body.extend_from_slice(OPTIONS);

        let len = body.len();
        body[..4].copy_from_slice(&[0, 0, 0, 2]);
        body[4..6].copy_from_slice(&((len - 6) as u16).to_be_bytes());
        body[6..10].copy_from_slice(&((len - 10) as u32).to_be_bytes());
        body
    }

    // 2^127 − 1은 소수입니다. SRP의 등식은 법이 소수이기만 하면 성립하므로, 그룹 상수를 옮겨 적지 않고 계산을 검증합니다.
    fn prime() -> BigUint {
        (BigUint::from(1u8) << 127usize) - BigUint::from(1u8)
    }

    // 실제 서버(2026-10-11 실측)가 보낸 챌린지의 머리 16바이트입니다. N은 `ff`로 시작합니다.
    const CAPTURED_HEAD: &str = "00000002048700000483000200ffffff";

    #[test]
    fn challenge_with_the_server_layout_is_1165_bytes() {
        let body = body(&prime(), &BigUint::from(7u8), &[1; 32], 1000);
        let head: String = body[..13].iter().map(|byte| format!("{byte:02x}")).collect();

        assert_eq!(body.len(), 1165);
        assert_eq!(head, CAPTURED_HEAD[..26]);
        assert!(Challenge::parse(&body).is_ok());
    }

    #[test]
    fn client_and_server_derive_the_same_key_and_proofs() {
        let n = prime();
        let g = BigUint::from(5u8);
        let salt = [3; 32];
        let server_secret = BigUint::from(0x1234_5678_9abc_def0u64);

        // 서버는 저장해 둔 검증값 v = g^x로 B = k·v + g^b를 보냅니다.
        let placeholder = Challenge::parse(&body(&n, &BigUint::from(7u8), &salt, 10)).unwrap();
        let x = placeholder.secret("비밀번호");
        let v = g.modpow(&x, &n);
        let k = number_hash(&[&pad(&n), &pad(&g)]);
        let b = (&k * &v + g.modpow(&server_secret, &n)) % &n;

        let challenge = Challenge::parse(&body(&n, &b, &salt, 10)).unwrap();
        let proof = challenge.respond_with("비밀번호", &BigUint::from(0x42u8));

        // 서버 쪽 S = (A · v^u)^b
        let a = BigUint::from_bytes_be(&proof.packet[20..532]);
        let u = number_hash(&[&pad(&a), &pad(&b)]);
        let s = (&a * v.modpow(&u, &n)).modpow(&server_secret, &n);
        let key = hash(&[&pad(&s)]);

        assert_eq!(proof.wrap_key, Sha256::digest(key)[..16]);

        let m1 = &proof.packet[533..597];
        let mut block = vec![0; 9];
        block.push(64);
        block.extend_from_slice(&hash(&[&pad(&a), m1, &key]));
        assert!(proof.verify(&block));
        assert!(!proof.verify(&block[..block.len() - 1]));
    }

    #[test]
    fn proof_packet_is_1080_bytes_with_a_678_byte_inner_body() {
        let challenge = Challenge::parse(&body(&prime(), &BigUint::from(7u8), &[1; 32], 10)).unwrap();
        let packet = challenge.respond("pw").packet;

        assert_eq!(packet.len(), 1080);
        assert_eq!(&packet[..14], b"\x00\x00\x04\x34\x01\x00RSA1\x00\x02\x02\xaa");
        assert_eq!(&packet[14..18], b"\x00\x00\x02\xa6");
        assert_eq!(&packet[18..20], b"\x02\x00");
        assert_eq!(&packet[597..599], &(OPTIONS.len() as u16).to_be_bytes());
        assert_eq!(&packet[599..679], OPTIONS);
        assert_eq!(packet[679], 16);
        assert!(packet[696..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn short_challenge_means_the_server_left_srp() {
        assert!(matches!(Challenge::parse(&[0; 999]), Err(Error::Unsupported)));
    }

    #[test]
    fn excessive_iterations_are_refused() {
        let body = body(&prime(), &BigUint::from(7u8), &[1; 32], 1_000_001);
        assert!(Challenge::parse(&body).is_err());
    }

    #[test]
    fn public_value_congruent_to_zero_is_refused() {
        let n = prime();
        assert!(Challenge::parse(&body(&n, &n, &[1; 32], 10)).is_err());
    }

    #[test]
    fn truncated_challenge_is_an_error_at_every_cut() {
        let body = body(&prime(), &BigUint::from(7u8), &[1; 32], 10);

        for cut in 1000..body.len() {
            assert!(Challenge::parse(&body[..cut]).is_err(), "cut at {cut}");
        }
    }
}
