//! 보안 유형 33(RSA-SRP) 인증입니다.
//!
//! - 서버의 RSA 공개키로 사용자 이름을 암호화해 보낸 뒤, 비밀번호는 SRP-6a(RFC 5054)로 증명합니다. 비밀번호 자체는 전송하지 않습니다.
//! - 서버의 공개키에는 인증서가 없습니다. 처음 본 키를 기억해 두고, 바뀌면 접속을 멈추는 판단은 호출자가 합니다.
//! - 비SRP 경로(사용자 이름과 비밀번호를 대칭키로만 보호해 보내는 경로)는 구현하지 않습니다.
use crate::error::{Error, Result};
use crate::reader::Reader;
use rand::rngs::OsRng;
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
pub use srp::{Challenge, Proof};

mod srp;

/// 보안 유형 목록에서 고르는 RSA-SRP 유형
pub const SECURITY_TYPE: u8 = 33;

/// 인증의 첫 메시지. 보안 유형을 고르면서 서버의 RSA 공개키를 요청합니다.
pub const KEY_REQUEST: [u8; 15] = *b"\x21\x00\x00\x00\x0a\x01\x00RSA1\x00\x00\x00\x00";

// 서버가 보내는 메시지는 모두 `u32` 길이 접두가 붙습니다. 키 응답은 수백 바이트, SRP 챌린지는 약 1.2 KiB입니다.
pub const MAX_MESSAGE_BYTES: usize = 8 << 10;
const MAX_KEY_BYTES: usize = 2 << 10;

// RSA-2048 암호문의 길이와, 그 뒤를 0으로 채운 사용자 이름 패킷의 길이입니다.
const CIPHERTEXT_BYTES: usize = 256;
const IDENTITY_BYTES: usize = 654;

/// 서버의 RSA 공개키입니다.
pub struct ServerKey {
    der: Vec<u8>,
    key: RsaPublicKey,
}

impl ServerKey {
    /// 키 요청에 대한 응답 본문(길이 접두를 뗀 것)을 해석합니다.
    pub fn parse(body: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(body, "서버 공개키");
        reader.skip(2)?;

        let len = reader.u32()? as usize;
        let der = reader.prefixed(len, MAX_KEY_BYTES)?;
        let key = RsaPublicKey::from_public_key_der(der).map_err(|_| Error::Malformed("서버 공개키"))?;

        Ok(Self { der: der.to_vec(), key })
    }

    /// DER 형식의 `SubjectPublicKeyInfo`. 처음 접속했을 때 저장해 두고 다음 접속에서 비교합니다.
    pub fn der(&self) -> &[u8] {
        &self.der
    }

    /// 사용자 이름을 서버의 공개키로 암호화한 SRP 시작 메시지를 만듭니다.
    pub fn identity(&self, username: &str) -> Result<Vec<u8>> {
        let name = username.as_bytes();

        // PKCS#1 v1.5로 RSA-2048에 담을 수 있는 평문은 245바이트입니다.
        if name.len() > 234 {
            return Err(Error::TooLong {
                what: "사용자 이름",
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
            .map_err(|_| Error::Malformed("서버 공개키"))?;

        if ciphertext.len() != CIPHERTEXT_BYTES {
            return Err(Error::Malformed("서버 공개키"));
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

    // 서버의 키 응답 본문과 같은 모양을 만듭니다.
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

        // 끝의 0 한 바이트는 읽지 않으므로 그 앞까지만 자릅니다.
        for cut in 0..body.len() - 1 {
            assert!(ServerKey::parse(&body[..cut]).is_err(), "cut at {cut}");
        }
    }
}
