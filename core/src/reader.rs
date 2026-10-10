use crate::error::{Error, Result};

// 서버가 보낸 바이트를 앞에서부터 읽습니다. 모든 읽기는 남은 길이를 먼저 확인하므로 패닉하지 않습니다.
pub struct Reader<'a> {
    buf: &'a [u8],
    what: &'static str,
}

impl<'a> Reader<'a> {
    // `what`은 잘렸을 때 에러에 실을 메시지 이름입니다.
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
        Ok(bytes.try_into().expect("길이를 확인했다"))
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

    pub fn f64(&mut self) -> Result<f64> {
        self.array().map(f64::from_be_bytes)
    }

    // 길이 접두 뒤의 본문을 읽습니다. 길이가 `max`를 넘으면 할당하지 않고 실패합니다.
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
