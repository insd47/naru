//! Record layer that wraps the control channel after authentication.
//!
//! - Records are AES-128-CBC, and the CBC state carries over between records for the whole session in each direction.
//! - The integrity tag is a plain SHA-1 over a per-direction sequence number and the plaintext, not an HMAC.
use crate::error::{Error, Result};
use crate::reader::Reader;
use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, BlockDecryptMut, BlockEncryptMut, KeyInit, KeyIvInit};
use sha1::{Digest, Sha1};

const BLOCK_BYTES: usize = 16;
const TAG_BYTES: usize = 20;

// The ciphertext length is a `u16`, so one record's plaintext can't exceed 65520 bytes (a multiple of 16).
const MAX_CIPHERTEXT_BYTES: usize = u16::MAX as usize / BLOCK_BYTES * BLOCK_BYTES;
const MAX_BODY_BYTES: usize = MAX_CIPHERTEXT_BYTES - 2 - TAG_BYTES;

// When the tag doesn't match, nearby sequence numbers are tried in this order, because the server's sequence number has been seen to drift.
const SEQUENCE_WINDOW: [i64; 7] = [0, -1, 1, 2, 3, 4, 5];

/// Record layer keys unwrapped from the key exchange message. Both directions start with the same key and IV.
#[derive(Clone)]
pub struct RecordKeys {
    /// AES-128 key, also used to encrypt pointer events
    pub content_key: [u8; 16],
    /// Initial IV for both directions
    pub iv: [u8; 16],
}

impl RecordKeys {
    /// Unwraps the 36-byte body of the key exchange rectangle with the wrapping key from authentication.
    ///
    /// - The key and IV after the `u32` generation number are each encrypted with AES-128-ECB.
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

/// Wraps outgoing messages in records.
///
/// - Sequence numbers and CBC state carry over, so records must be sent in the order they were sealed.
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

    /// Turns one message into wire bytes, splitting it across consecutive records if it doesn't fit in one.
    pub fn seal(&mut self, message: &[u8]) -> Vec<u8> {
        let mut wire = Vec::with_capacity(message.len() + 64);

        // An empty message still goes out as one record.
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

/// Opens received bytes record by record, regardless of TCP read boundaries.
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

    /// Takes out the body of the next complete record, if any.
    ///
    /// - Returns `Integrity` when the tag matches no sequence number near the expected one; the connection must be dropped.
    pub fn pop(&mut self) -> Result<Option<Vec<u8>>> {
        let Some(&[high, low]) = self.buffer.get(..2) else {
            return Ok(None);
        };

        let len = u16::from_be_bytes([high, low]) as usize;

        if len == 0 || !len.is_multiple_of(BLOCK_BYTES) || len < 2 + TAG_BYTES {
            return Err(Error::Malformed("record"));
        }

        if self.buffer.len() < 2 + len {
            return Ok(None);
        }

        let mut plain: Vec<u8> = self.buffer.drain(..2 + len).skip(2).collect();

        // CBC state must advance whatever the tag result, or the next record can't be decrypted.
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

        let mut reader = Reader::new(content, "record");
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

        // Decrypting the second record with the initial IV corrupts its first block, so the tag fails.
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
        // The receiver expects sequence number 0, but the server starts at 3.
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
