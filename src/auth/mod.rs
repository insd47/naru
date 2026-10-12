//! Security type 33 (RSA-SRP) authentication.
//!
//! - The username is sent encrypted with the server's RSA public key, then the password is proven with SRP-6a (RFC 5054). The password itself is never sent.
//! - The server's public key has no certificate. The caller remembers the first key it sees and decides to stop connecting if it changes.
//! - The non-SRP path (username and password protected only by a symmetric key) is not implemented.
use crate::error::{Error, Result};
use crate::reader::Reader;
use rand::rngs::OsRng;
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
pub use srp::{Challenge, Proof};

mod srp;

/// RFB version the client sends.
pub const VERSION: &[u8; 12] = b"RFB 003.889\n";

/// RSA-SRP type picked from the security type list
pub const SECURITY_TYPE: u8 = 33;

/// First authentication message. It picks the security type and requests the server's RSA public key.
pub const KEY_REQUEST: [u8; 15] = *b"\x21\x00\x00\x00\x0a\x01\x00RSA1\x00\x00\x00\x00";

// Every server message has a `u32` length prefix. The key response is a few hundred bytes and the SRP challenge about 1.2 KiB.
pub const MAX_MESSAGE_BYTES: usize = 8 << 10;
const MAX_KEY_BYTES: usize = 2 << 10;

// Length of an RSA-2048 ciphertext, and of the zero-padded username packet that carries it.
const CIPHERTEXT_BYTES: usize = 256;
const IDENTITY_BYTES: usize = 654;

/// The server's RSA public key.
pub struct ServerKey {
    der: Vec<u8>,
    key: RsaPublicKey,
}

impl ServerKey {
    /// Parses the key response body, without its length prefix.
    pub fn parse(body: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(body, "public key");
        reader.skip(2)?;

        let len = reader.u32()? as usize;
        let der = reader.prefixed(len, MAX_KEY_BYTES)?;
        let key = RsaPublicKey::from_public_key_der(der).map_err(|_| Error::Malformed("public key"))?;

        Ok(Self { der: der.to_vec(), key })
    }

    /// DER-encoded `SubjectPublicKeyInfo`. Store it on the first connection and compare it on later ones.
    pub fn der(&self) -> &[u8] {
        &self.der
    }

    /// Builds the SRP start message with the username encrypted under the server's public key.
    pub fn identity(&self, username: &str) -> Result<Vec<u8>> {
        let name = username.as_bytes();

        // PKCS#1 v1.5 fits at most 245 bytes of plaintext in RSA-2048.
        if name.len() > 234 {
            return Err(Error::TooLong {
                what: "username",
                len: name.len(),
            });
        }

        let mut plain = Vec::with_capacity(name.len() + 11);
        plain.extend_from_slice(&(name.len() as u32 + 7).to_be_bytes());
        plain.extend_from_slice(&(name.len() as u32).to_be_bytes());
        plain.extend_from_slice(name);
        plain.extend_from_slice(&[0; 3]);

        let ciphertext = self
            .key
            .encrypt(&mut OsRng, Pkcs1v15Encrypt, &plain)
            .map_err(|_| Error::Malformed("public key"))?;

        if ciphertext.len() != CIPHERTEXT_BYTES {
            return Err(Error::Malformed("public key"));
        }

        let mut packet = Vec::with_capacity(IDENTITY_BYTES);
        packet.extend_from_slice(&(IDENTITY_BYTES as u32 - 4).to_be_bytes());
        packet.extend_from_slice(&[1, 0, b'R', b'S', b'A', b'1', 0, 2]);
        packet.extend_from_slice(&(CIPHERTEXT_BYTES as u16).to_be_bytes());
        packet.extend_from_slice(&ciphertext);
        packet.resize(IDENTITY_BYTES, 0);
        Ok(packet)
    }
}

#[cfg(test)]
mod tests {
    use super::ServerKey;
    use rand::rngs::OsRng;
    use rsa::pkcs8::EncodePublicKey;
    use rsa::{Pkcs1v15Encrypt, RsaPrivateKey};

    // Builds a body shaped like the server's key response.
    fn response(private: &RsaPrivateKey) -> Vec<u8> {
        let der = private.to_public_key().to_public_key_der().unwrap();
        let der = der.as_bytes();

        let mut body = vec![0, 1];
        body.extend_from_slice(&(der.len() as u32).to_be_bytes());
        body.extend_from_slice(der);
        body.push(0);
        body
    }

    #[test]
    fn identity_packet_is_654_bytes_and_carries_the_encrypted_name() {
        let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
        let key = ServerKey::parse(&response(&private)).unwrap();
        let packet = key.identity("insd").unwrap();

        assert_eq!(packet.len(), 654);
        assert_eq!(&packet[..14], b"\x00\x00\x02\x8a\x01\x00RSA1\x00\x02\x01\x00");
        assert!(packet[270..].iter().all(|&byte| byte == 0));

        let plain = private.decrypt(Pkcs1v15Encrypt, &packet[14..270]).unwrap();
        assert_eq!(plain, b"\x00\x00\x00\x0b\x00\x00\x00\x04insd\x00\x00\x00");
    }

    #[test]
    fn oversized_key_length_is_rejected_without_allocating() {
        let body = [0, 1, 0xff, 0xff, 0xff, 0xff];
        assert!(ServerKey::parse(&body).is_err());
    }

    #[test]
    fn truncated_key_response_is_an_error_at_every_cut() {
        let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
        let body = response(&private);

        // The trailing zero byte is never read, so cuts stop before it.
        for cut in 0..body.len() - 1 {
            assert!(ServerKey::parse(&body[..cut]).is_err(), "cut at {cut}");
        }
    }
}
