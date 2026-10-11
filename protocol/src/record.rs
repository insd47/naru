//! 인증 뒤 제어 채널을 감싸는 레코드 계층입니다.
//!
//! - 레코드는 `u16` 암호문 길이와 AES-128-CBC 암호문입니다. 평문은 `u16` 본문 길이, 본문, 채움, SHA-1 태그 20바이트입니다.
//! - CBC 상태는 방향마다 세션 내내 이어집니다. 한 레코드의 마지막 암호문 블록이 다음 레코드의 IV입니다.
//! - 태그는 `SHA-1(u32 순번 || 태그 앞까지의 평문)`이며 HMAC이 아닙니다. 순번은 방향마다 0부터 셉니다.
use crate::error::{Error, Result};
use crate::reader::Reader;
use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, BlockDecryptMut, BlockEncryptMut, KeyInit, KeyIvInit};
use sha1::{Digest, Sha1};

const BLOCK_BYTES: usize = 16;
const TAG_BYTES: usize = 20;

// 암호문 길이는 `u16`이므로 한 레코드의 평문은 65520바이트(16의 배수)를 넘지 못합니다.
const MAX_CIPHERTEXT_BYTES: usize = u16::MAX as usize / BLOCK_BYTES * BLOCK_BYTES;
const MAX_BODY_BYTES: usize = MAX_CIPHERTEXT_BYTES - 2 - TAG_BYTES;

// 태그가 맞지 않으면 기대한 순번의 앞뒤를 이 순서로 시도합니다. 서버의 순번이 어긋나는 경우가 관측되었습니다.
const SEQUENCE_WINDOW: [i64; 7] = [0, -1, 1, 2, 3, 4, 5];

/// 키 교체 메시지에서 푼 레코드 계층의 키입니다. 양방향이 같은 키와 같은 IV로 시작합니다.
#[derive(Clone)]
pub struct RecordKeys {
    /// AES-128 키. 포인터 이벤트의 암호화에도 씁니다.
    pub content_key: [u8; 16],
    /// 두 방향의 첫 IV
    pub iv: [u8; 16],
}

impl RecordKeys {
    /// 키 교체 사각형의 36바이트 본문을 인증에서 얻은 래핑 키로 풉니다.
    ///
    /// - 본문은 `u32` 세대 번호, 암호화된 키, 암호화된 IV이며, 키와 IV는 각각 AES-128-ECB로 암호화되어 있습니다.
    pub fn unwrap(body: &[u8; 36], wrap_key: &[u8; 16]) -> Self {
        let cipher = Aes128::new(wrap_key.into());

        let decrypt = |encrypted: &[u8]| {
            let mut block = GenericArray::clone_from_slice(encrypted);
            cipher.decrypt_block(&mut block);
            block.into()
        };

        Self {
            content_key: decrypt(&body[4..20]),
            iv: decrypt(&body[20..36]),
        }
    }
}

/// 보내는 메시지를 레코드로 감쌉니다.
///
/// - 순번과 CBC 상태가 이어지므로, 만든 레코드는 만든 순서대로 보내야 합니다.
pub struct Sealer {
    cipher: cbc::Encryptor<Aes128>,
    sequence: u32,
}

impl Sealer {
    pub fn new(keys: &RecordKeys) -> Self {
        Self {
            cipher: cbc::Encryptor::new(&keys.content_key.into(), &keys.iv.into()),
            sequence: 0,
        }
    }

    /// 메시지 하나를 전송할 바이트로 만듭니다. 한 레코드에 담을 수 없으면 이어지는 레코드로 나눕니다.
    pub fn seal(&mut self, message: &[u8]) -> Vec<u8> {
        let mut wire = Vec::with_capacity(message.len() + 64);

        // 빈 메시지도 레코드 하나로 보냅니다.
        let empty: &[&[u8]] = if message.is_empty() { &[&[]] } else { &[] };

        for body in message.chunks(MAX_BODY_BYTES).chain(empty.iter().copied()) {
            let plain_len = (2 + body.len() + TAG_BYTES).next_multiple_of(BLOCK_BYTES);

            let mut plain = Vec::with_capacity(plain_len);
            plain.extend_from_slice(&(body.len() as u16).to_be_bytes());
            plain.extend_from_slice(body);
            plain.resize(plain_len - TAG_BYTES, 0);
            plain.extend_from_slice(&tag(self.sequence, &plain));
            self.sequence = self.sequence.wrapping_add(1);

            for block in plain.as_chunks_mut::<BLOCK_BYTES>().0 {
                self.cipher.encrypt_block_mut(GenericArray::from_mut_slice(block));
            }

            wire.extend_from_slice(&(plain_len as u16).to_be_bytes());
            wire.extend_from_slice(&plain);
        }

        wire
    }
}

/// 받은 바이트를 TCP 읽기 경계와 관계없이 레코드 단위로 풉니다.
pub struct Opener {
    cipher: cbc::Decryptor<Aes128>,
    sequence: u32,
    buffer: Vec<u8>,
}

impl Opener {
    pub fn new(keys: &RecordKeys) -> Self {
        Self {
            cipher: cbc::Decryptor::new(&keys.content_key.into(), &keys.iv.into()),
            sequence: 0,
            buffer: Vec::new(),
        }
    }

    pub fn push(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
    }

    /// 완성된 레코드가 있으면 본문을 꺼냅니다.
    ///
    /// - 태그가 기대한 순번 근처의 어느 값과도 맞지 않으면 `Integrity`이며, 연결을 끊어야 합니다.
    pub fn pop(&mut self) -> Result<Option<Vec<u8>>> {
        let Some(&[high, low]) = self.buffer.get(..2) else {
            return Ok(None);
        };

        let len = u16::from_be_bytes([high, low]) as usize;

        if len == 0 || !len.is_multiple_of(BLOCK_BYTES) || len < 2 + TAG_BYTES {
            return Err(Error::Malformed("레코드"));
        }

        if self.buffer.len() < 2 + len {
            return Ok(None);
        }

        let mut plain: Vec<u8> = self.buffer.drain(..2 + len).skip(2).collect();

        // 태그 결과와 관계없이 CBC 상태는 항상 전진해야 다음 레코드를 풀 수 있습니다.
        for block in plain.as_chunks_mut::<BLOCK_BYTES>().0 {
            self.cipher.decrypt_block_mut(GenericArray::from_mut_slice(block));
        }

        let (content, expected) = plain.split_at(len - TAG_BYTES);

        let matched = SEQUENCE_WINDOW.iter().find_map(|offset| {
            let sequence = (self.sequence as i64 + offset) as u32;
            (tag(sequence, content) == expected).then_some(sequence)
        });

        let Some(sequence) = matched else {
            return Err(Error::Integrity);
        };

        self.sequence = sequence.wrapping_add(1);

        let mut reader = Reader::new(content, "레코드");
        let body_len = reader.u16()? as usize;
        Ok(Some(reader.bytes(body_len)?.to_vec()))
    }
}

fn tag(sequence: u32, content: &[u8]) -> [u8; TAG_BYTES] {
    let mut digest = Sha1::new();
    digest.update(sequence.to_be_bytes());
    digest.update(content);
    digest.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::{MAX_BODY_BYTES, Opener, RecordKeys, Sealer};
    use crate::error::Error;
    use aes::Aes128;
    use aes::cipher::generic_array::GenericArray;
    use aes::cipher::{BlockEncrypt, KeyInit};

    fn keys() -> RecordKeys {
        RecordKeys {
            content_key: [7; 16],
            iv: [9; 16],
        }
    }

    fn open_all(opener: &mut Opener) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| opener.pop().unwrap()).collect()
    }

    #[test]
    fn key_exchange_body_unwraps_key_and_iv_separately() {
        let wrap_key = [1; 16];
        let cipher = Aes128::new(&wrap_key.into());
        let mut body = [0; 36];
        body[..4].copy_from_slice(&1u32.to_be_bytes());

        for (range, value) in [(4..20, [0xaa; 16]), (20..36, [0xbb; 16])] {
            let mut block = GenericArray::from(value);
            cipher.encrypt_block(&mut block);
            body[range].copy_from_slice(&block);
        }

        let keys = RecordKeys::unwrap(&body, &wrap_key);
        assert_eq!((keys.content_key, keys.iv), ([0xaa; 16], [0xbb; 16]));
    }

    #[test]
    fn records_are_whole_blocks_with_room_for_the_tag() {
        let mut sealer = Sealer::new(&keys());

        for (body, ciphertext) in [(0, 32), (10, 32), (11, 48), (26, 48), (27, 64)] {
            let wire = sealer.seal(&vec![0; body]);
            assert_eq!(wire.len(), 2 + ciphertext, "body {body}");
            assert_eq!(u16::from_be_bytes([wire[0], wire[1]]) as usize, ciphertext);
        }
    }

    #[test]
    fn cbc_state_carries_across_records() {
        let mut sealer = Sealer::new(&keys());
        let first = sealer.seal(b"first");
        let second = sealer.seal(b"second");

        let mut opener = Opener::new(&keys());
        opener.push(&first);
        opener.push(&second);
        assert_eq!(open_all(&mut opener), [b"first".to_vec(), b"second".to_vec()]);

        // 두 번째 레코드를 처음 IV로 풀면 첫 블록이 깨져 태그가 맞지 않습니다.
        let mut restarted = Opener::new(&keys());
        restarted.push(&second);
        assert!(matches!(restarted.pop(), Err(Error::Integrity)));
    }

    #[test]
    fn every_split_point_yields_the_same_records() {
        let mut sealer = Sealer::new(&keys());
        let wire = [sealer.seal(b"alpha"), sealer.seal(&[0x55; 300]), sealer.seal(b"")].concat();

        let mut whole = Opener::new(&keys());
        whole.push(&wire);
        let expected = open_all(&mut whole);
        assert_eq!(expected.len(), 3);

        for cut in 0..=wire.len() {
            let mut opener = Opener::new(&keys());
            opener.push(&wire[..cut]);
            let mut records = open_all(&mut opener);
            opener.push(&wire[cut..]);
            records.extend(open_all(&mut opener));
            assert_eq!(records, expected, "cut at {cut}");
        }
    }

    #[test]
    fn sequence_skew_within_the_window_resynchronizes() {
        // 수신 측은 순번 0을 기대하는데 서버가 3부터 보낸 경우입니다.
        let mut sealer = Sealer::new(&keys());
        sealer.sequence = 3;
        let wire: Vec<u8> = (0..4).flat_map(|index| sealer.seal(&[index])).collect();

        let mut opener = Opener::new(&keys());
        opener.push(&wire);
        assert_eq!(open_all(&mut opener), [[0], [1], [2], [3]]);
        assert_eq!(opener.sequence, 7);
    }

    #[test]
    fn sequence_skew_outside_the_window_is_an_integrity_failure() {
        let mut sealer = Sealer::new(&keys());
        sealer.sequence = 6;

        let mut opener = Opener::new(&keys());
        opener.push(&sealer.seal(b"late"));
        assert!(matches!(opener.pop(), Err(Error::Integrity)));
    }

    #[test]
    fn tampered_ciphertext_fails_the_tag() {
        let mut wire = Sealer::new(&keys()).seal(b"pointer");
        let last = wire.len() - 1;
        wire[last] ^= 1;

        let mut opener = Opener::new(&keys());
        opener.push(&wire);
        assert!(matches!(opener.pop(), Err(Error::Integrity)));
    }

    #[test]
    fn long_message_spans_records_and_reassembles() {
        let message: Vec<u8> = (0..MAX_BODY_BYTES * 2 + 5).map(|at| at as u8).collect();
        let wire = Sealer::new(&keys()).seal(&message);

        let mut opener = Opener::new(&keys());
        opener.push(&wire);
        let records = open_all(&mut opener);

        assert_eq!(records.len(), 3);
        assert_eq!(records.concat(), message);
    }

    #[test]
    fn zero_or_unaligned_length_is_malformed() {
        for header in [[0, 0], [0, 17]] {
            let mut opener = Opener::new(&keys());
            opener.push(&header);
            assert!(matches!(opener.pop(), Err(Error::Malformed(_))));
        }
    }
}
