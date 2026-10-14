use crate::error::{Error, Result};
use crate::reader::Reader;
use num_bigint_dig::BigUint;
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256, Sha512};

// The group is a 4096-bit MODP prime, so every big number is padded to 512 bytes before hashing.
const PAD_BYTES: usize = 512;

// A server that falls back to the non-SRP path sends a shorter body than this.
const MIN_CHALLENGE_BYTES: usize = 1000;

// The PBKDF2 iteration count varies per account. It is capped so a malicious server can't stall the client.
const MAX_ITERATIONS: u64 = 1_000_000;

// The proof message is a fixed 1076 bytes after its length prefix: a 14-byte header, the content, then zero padding.
const PROOF_BYTES: usize = 1076;
const PROOF_HEADER_BYTES: usize = 14;

/// SRP challenge from the server.
pub struct Challenge {
    n: BigUint,
    g: BigUint,
    salt: Vec<u8>,
    b: BigUint,
    iterations: u32,
    // String naming the hash, cipher, and KDF the server chose. It is echoed back unchanged in the proof message.
    options: Vec<u8>,
}

impl Challenge {
    /// Parses the challenge body, without its length prefix.
    ///
    /// - A short body means the server chose the non-SRP path, so this returns `Unsupported`.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < MIN_CHALLENGE_BYTES {
            return Err(Error::Unsupported);
        }

        // The 11-byte header (`00 00 00 02`, two lengths, a control byte) is unused.
        let mut reader = Reader::new(body, "SRP challenge");
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
            return Err(Error::Malformed("SRP iteration count"));
        }

        // A public value congruent to zero pins the shared secret to zero (RFC 5054 2.5.4).
        if n.bits() < 2 || &b % &n == BigUint::default() || &g % &n == BigUint::default() {
            return Err(Error::Malformed("SRP challenge"));
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

    /// Computes the proof from the password.
    ///
    /// - The PBKDF2 iterations can take a few hundred milliseconds.
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

        // S = (B - k·g^x)^(a + u·x) mod N. N is added before the reduction so the subtraction never goes negative.
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

        let wrap_key = Sha256::digest(key)[..16].try_into().expect("SHA-256 is 32 bytes");

        Proof {
            packet,
            wrap_key,
            server_proof: m2,
        }
    }

    // x = H(salt || H(":" || PBKDF2(password))). The username slot is empty.
    fn secret(&self, password: &str) -> BigUint {
        let mut dk = [0; 128];
        pbkdf2::pbkdf2_hmac::<Sha512>(password.as_bytes(), &self.salt, self.iterations, &mut dk);
        number_hash(&[&self.salt, &hash(&[b":", &dk])])
    }
}

/// Password proof, plus the key used after authentication succeeds.
pub struct Proof {
    /// Proof message to send to the server (1080 bytes)
    pub packet: Vec<u8>,
    /// AES-128 key that unwraps the first key exchange message
    pub wrap_key: [u8; 16],
    server_proof: [u8; 64],
}

impl Proof {
    /// Checks that the block sent with the authentication result contains the expected server proof (M2).
    ///
    /// - M2's offset within the block isn't confirmed, so the whole block is searched.
    pub fn verify(&self, block: &[u8]) -> bool {
        block
            .windows(self.server_proof.len())
            .any(|window| window == self.server_proof)
    }
}

// Reads a big number after stripping leading zeros. Values longer than 512 bytes are accepted if the excess is leading
// zeros.
fn number(bytes: &[u8]) -> Result<BigUint> {
    let start = bytes.iter().position(|&byte| byte != 0).unwrap_or(bytes.len());

    if bytes.len() - start > PAD_BYTES {
        return Err(Error::Malformed("SRP challenge"));
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
    use sha2::{Digest, Sha256};

    // Options string shaped like the one the real server sends.
    const OPTIONS: &[u8] = b"mda=SHA-512,replay_detection,conf+int=ChaCha20-Poly1305,kdf=SALTED-SHA512-PBKDF2";

    // Builds a body laid out like the real server's challenge.
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

    // 2^127 - 1 is prime. SRP's identities hold for any prime modulus, so this checks the math without copying the
    // group constant.
    fn prime() -> BigUint {
        (BigUint::from(1u8) << 127usize) - BigUint::from(1u8)
    }

    // First 16 bytes of a challenge captured from a real server. N starts with `ff`.
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

        // The server sends B = k·v + g^b using its stored verifier v = g^x.
        let placeholder = Challenge::parse(&body(&n, &BigUint::from(7u8), &salt, 10)).unwrap();
        let x = placeholder.secret("비밀번호");
        let v = g.modpow(&x, &n);
        let k = number_hash(&[&pad(&n), &pad(&g)]);
        let b = (&k * &v + g.modpow(&server_secret, &n)) % &n;

        let challenge = Challenge::parse(&body(&n, &b, &salt, 10)).unwrap();
        let proof = challenge.respond_with("비밀번호", &BigUint::from(0x42u8));

        // Server side: S = (A · v^u)^b
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
