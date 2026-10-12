use crate::error::{Error, Result};

// Reads server bytes front to back. Every read checks the remaining length first, so it never panics.
pub struct Reader<'a> {
    buf: &'a [u8],
    what: &'static str,
}

impl<'a> Reader<'a> {
    // `what` names the message in the error when the input is truncated.
    pub fn new(buf: &'a [u8], what: &'static str) -> Self {
        Self { buf, what }
    }

    pub fn bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let Some((head, rest)) = self.buf.split_at_checked(len) else {
            return Err(Error::Truncated(self.what));
        };

        self.buf = rest;
        Ok(head)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let bytes = self.bytes(N)?;
        Ok(bytes.try_into().expect("length was checked"))
    }

    pub fn skip(&mut self, len: usize) -> Result<()> {
        self.bytes(len).map(drop)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        self.array().map(u16::from_be_bytes)
    }

    pub fn u32(&mut self) -> Result<u32> {
        self.array().map(u32::from_be_bytes)
    }

    pub fn i32(&mut self) -> Result<i32> {
        self.array().map(i32::from_be_bytes)
    }

    pub fn u64(&mut self) -> Result<u64> {
        self.array().map(u64::from_be_bytes)
    }

    // Reads a length-prefixed body. Fails without allocating if the length exceeds `max`.
    pub fn prefixed(&mut self, len: usize, max: usize) -> Result<&'a [u8]> {
        if len > max {
            return Err(Error::TooLong { what: self.what, len });
        }

        self.bytes(len)
    }

    pub fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.buf)
    }

    pub fn remaining(&self) -> usize {
        self.buf.len()
    }
}
