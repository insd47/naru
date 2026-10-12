use crate::error::{Error, Result};
use crate::reader::Reader;

const MAX_DISPLAYS: u16 = 16;
const DISPLAY_BYTES: usize = 56;

/// Mac의 디스플레이 레이아웃입니다. 여러 모니터는 하나의 합쳐진 캔버스 안에 각자의 영역을 가집니다.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Mac이 UI를 그리는 좌표계의 캔버스 크기
    pub logical: (u16, u16),
    /// 디코딩된 영상의 크기. 포인터 좌표도 이 좌표계를 씁니다.
    pub pixels: (u16, u16),
}

impl Layout {
    /// 레이아웃 사각형의 본문(`u16` 길이를 뗀 것)을 해석합니다.
    ///
    /// - 디스플레이마다 배율 두 개, ID, 논리 영역과 픽셀 영역(각각 y0, x0, y1, x1 순)이 56바이트로 이어집니다.
    ///   캔버스 크기만 쓰므로 디스플레이 목록은 길이만 확인하고 건너뜁니다.
    pub fn parse(body: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(body, "디스플레이 레이아웃");
        reader.skip(2)?;

        let logical = (reader.u16()?, reader.u16()?);
        let pixels = (reader.u16()?, reader.u16()?);
        reader.skip(8)?;

        let count = reader.u16()?;

        if count > MAX_DISPLAYS {
            return Err(Error::Malformed("디스플레이 레이아웃"));
        }

        reader.skip(usize::from(count) * DISPLAY_BYTES)?;
        Ok(Self { logical, pixels })
    }
}

#[cfg(test)]
mod tests {
    use super::Layout;

    // 논리 1512×982, 픽셀 3024×1964인 Retina 디스플레이 하나의 레이아웃입니다.
    fn body() -> Vec<u8> {
        let mut body = Vec::new();

        for value in [5u16, 1512, 982, 3024, 1964] {
            body.extend_from_slice(&value.to_be_bytes());
        }

        body.extend_from_slice(&[0xff; 4]);
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&1u16.to_be_bytes());
        body.extend_from_slice(&2f64.to_be_bytes());
        body.extend_from_slice(&2f64.to_be_bytes());
        body.extend_from_slice(&1u32.to_be_bytes());

        for value in [0u16, 0, 982, 1512, 0, 0, 1964, 3024] {
            body.extend_from_slice(&value.to_be_bytes());
        }

        body.extend_from_slice(&[0; 20]);
        body
    }

    #[test]
    fn canvas_sizes_are_read_and_displays_skipped() {
        let layout = Layout::parse(&body()).unwrap();
        assert_eq!((layout.logical, layout.pixels), ((1512, 982), (3024, 1964)));
    }

    #[test]
    fn truncated_layout_is_an_error_at_every_cut() {
        let body = body();

        for cut in 0..body.len() {
            assert!(Layout::parse(&body[..cut]).is_err(), "cut at {cut}");
        }
    }
}
