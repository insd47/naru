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
    pub displays: Vec<Display>,
}

/// 캔버스 안의 디스플레이 하나입니다.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    pub id: u32,
    /// 가로, 세로 배율. 2는 Retina입니다.
    pub scale: (f64, f64),
    pub logical: Area,
    pub pixels: Area,
}

/// 캔버스 안의 영역. 끝 좌표는 포함하지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub x0: u16,
    pub y0: u16,
    pub x1: u16,
    pub y1: u16,
}

impl Layout {
    /// 레이아웃 사각형의 본문(`u16` 길이를 뗀 것)을 해석합니다.
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

        let displays = (0..count)
            .map(|_| {
                let mut entry = Reader::new(reader.bytes(DISPLAY_BYTES)?, "디스플레이 레이아웃");
                let scale = (entry.f64()?, entry.f64()?);
                let id = entry.u32()?;

                Ok(Display {
                    id,
                    scale,
                    logical: Area::read(&mut entry)?,
                    pixels: Area::read(&mut entry)?,
                })
            })
            .collect::<Result<_>>()?;

        Ok(Self {
            logical,
            pixels,
            displays,
        })
    }
}

impl Area {
    // 서버는 영역을 y0, x0, y1, x1 순서로 보냅니다.
    fn read(reader: &mut Reader) -> Result<Self> {
        let y0 = reader.u16()?;
        let x0 = reader.u16()?;
        let y1 = reader.u16()?;
        let x1 = reader.u16()?;
        Ok(Self { x0, y0, x1, y1 })
    }
}

#[cfg(test)]
mod tests {
    use super::{Area, Layout};

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
    fn areas_arrive_y_first_and_are_reordered() {
        let layout = Layout::parse(&body()).unwrap();

        assert_eq!(layout.pixels, (3024, 1964));
        assert_eq!(layout.displays[0].scale, (2.0, 2.0));

        assert_eq!(
            layout.displays[0].pixels,
            Area {
                x0: 0,
                y0: 0,
                x1: 3024,
                y1: 1964
            }
        );
    }

    #[test]
    fn truncated_layout_is_an_error_at_every_cut() {
        let body = body();

        for cut in 0..body.len() {
            assert!(Layout::parse(&body[..cut]).is_err(), "cut at {cut}");
        }
    }
}
