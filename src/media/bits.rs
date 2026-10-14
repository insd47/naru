//! Bit reading for parameter sets and slice headers, shared by both codecs.

/// Reads RBSP bits front to back, skipping emulation prevention bytes (the 03 in 00 00 03).
pub struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
    bit: u8,
    zeros: u8,
}

impl<'a> Bits<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            at: 0,
            bit: 0,
            zeros: 0,
        }
    }

    pub fn bit(&mut self) -> Option<u32> {
        if self.bit == 0 {
            if self.zeros >= 2 && self.bytes.get(self.at) == Some(&3) {
                self.at += 1;
                self.zeros = 0;
            }

            let byte = *self.bytes.get(self.at)?;
            self.zeros = if byte == 0 { self.zeros + 1 } else { 0 };
        }

        let value = (self.bytes[self.at] >> (7 - self.bit)) & 1;
        self.bit += 1;

        if self.bit == 8 {
            self.bit = 0;
            self.at += 1;
        }

        Some(u32::from(value))
    }

    pub fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;

        while self.bit()? == 0 {
            zeros += 1;

            if zeros > 31 {
                return None;
            }
        }

        let mut value = 1u64;

        for _ in 0..zeros {
            value = value << 1 | u64::from(self.bit()?);
        }

        u32::try_from(value - 1).ok()
    }

    /// Reads `count` bits, up to 32, as an unsigned number.
    pub fn read(&mut self, count: u32) -> Option<u32> {
        (0..count).try_fold(0, |value, _| Some(value << 1 | self.bit()?))
    }

    pub fn skip(&mut self, count: u32) -> Option<()> {
        (0..count).try_for_each(|_| self.bit().map(drop))
    }

    /// Reads a signed Exp-Golomb number.
    pub fn se(&mut self) -> Option<i32> {
        let code = i64::from(self.ue()?);
        let value = if code % 2 == 1 { (code + 1) / 2 } else { -code / 2 };
        i32::try_from(value).ok()
    }
}
