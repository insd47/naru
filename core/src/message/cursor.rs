use super::zlib;
use crate::error::{Error, Result};
use flate2::Decompress;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;

// 서버가 매기는 캐시 번호는 세션 안에서만 의미가 있습니다. 오래된 모양부터 버립니다.
const CACHE_SIZE: usize = 64;
const MAX_SIDE: u16 = 512;

/// 커서 모양입니다.
#[derive(Debug, PartialEq)]
pub struct CursorImage {
    /// 이미지 안에서 포인터 위치에 해당하는 점(픽셀)
    pub hotspot: (u16, u16),
    pub width: u16,
    pub height: u16,
    /// 행 우선 RGBA 픽셀. 알파는 미리 곱하지 않은 값입니다.
    pub rgba: Vec<u8>,
}

/// 커서 사각형을 해석하고, 서버가 매긴 번호로 모양을 저장해 둡니다.
///
/// - 압축 데이터가 있는 사각형은 모양을 저장하면서 그 모양으로 바꾸고, 없는 사각형은 저장해 둔 모양으로 바꿉니다.
pub struct Cursors {
    shapes: HashMap<u32, Arc<CursorImage>>,
    order: VecDeque<u32>,
    // 이어지는 데이터가 이전 사각형의 zlib 스트림을 잇는 경우에 대비해 해제기를 유지합니다.
    stream: Decompress,
}

impl Cursors {
    /// 커서 사각형 하나를 적용해 바뀐 모양을 반환합니다.
    ///
    /// - 저장한 적 없는 번호를 고르라는 사각형이 가끔 옵니다. 이때는 `None`이며 직전 모양을 유지하면 됩니다.
    pub fn apply(
        &mut self,
        hotspot: (u16, u16),
        size: (u16, u16),
        cache_id: u32,
        data: &[u8],
    ) -> Result<Option<Arc<CursorImage>>> {
        if data.is_empty() {
            return Ok(self.shapes.get(&cache_id).cloned());
        }

        let (width, height) = size;

        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(Error::Malformed("커서 크기"));
        }

        if zlib::starts_stream(data) {
            self.stream = Decompress::new(true);
        }

        // BGRA 픽셀 평면 뒤에 알파 평면이 따로 옵니다. 알파는 뒤의 평면에서 가져옵니다.
        let pixels = usize::from(width) * usize::from(height);
        let raw = zlib::inflate(&mut self.stream, data, pixels * 5, "커서 이미지")?;
        let (bgra, alpha) = raw.split_at(pixels * 4);

        let rgba = bgra
            .as_chunks::<4>()
            .0
            .iter()
            .zip(alpha)
            .flat_map(|(pixel, &alpha)| [pixel[2], pixel[1], pixel[0], alpha])
            .collect();

        let image = Arc::new(CursorImage {
            hotspot,
            width,
            height,
            rgba,
        });

        if self.shapes.insert(cache_id, image.clone()).is_none() {
            self.order.push_back(cache_id);
        }

        while self.order.len() > CACHE_SIZE {
            let oldest = self.order.pop_front().expect("길이를 확인했다");
            self.shapes.remove(&oldest);
        }

        Ok(Some(image))
    }
}

impl Default for Cursors {
    fn default() -> Self {
        Self {
            shapes: HashMap::new(),
            order: VecDeque::new(),
            stream: Decompress::new(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cursors;
    use flate2::Compress;
    use flate2::Compression;
    use flate2::FlushCompress;

    fn compressed(compress: &mut Compress, raw: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(raw.len() + 64);
        compress.compress_vec(raw, &mut output, FlushCompress::Sync).unwrap();
        output
    }

    // 2×1 커서: 파랑, 빨강 픽셀과 알파 평면 128, 255
    const RAW: [u8; 10] = [255, 0, 0, 0, 0, 0, 255, 0, 128, 255];

    #[test]
    fn alpha_comes_from_the_trailing_plane() {
        let mut compress = Compress::new(Compression::default(), true);
        let data = compressed(&mut compress, &RAW);

        let image = Cursors::default().apply((1, 0), (2, 1), 1000, &data).unwrap().unwrap();
        assert_eq!(image.rgba, [0, 0, 255, 128, 255, 0, 0, 255]);
    }

    #[test]
    fn select_reuses_a_stored_shape_and_ignores_unknown_ids() {
        let mut compress = Compress::new(Compression::default(), true);
        let mut cursors = Cursors::default();
        let stored = cursors
            .apply((0, 0), (2, 1), 1000, &compressed(&mut compress, &RAW))
            .unwrap();

        assert_eq!(cursors.apply((0, 0), (0, 0), 1000, &[]).unwrap(), stored);
        assert_eq!(cursors.apply((0, 0), (0, 0), 1001, &[]).unwrap(), None);
    }

    #[test]
    fn continuation_of_the_previous_stream_inflates() {
        let mut compress = Compress::new(Compression::default(), true);
        let mut cursors = Cursors::default();
        cursors
            .apply((0, 0), (2, 1), 1000, &compressed(&mut compress, &RAW))
            .unwrap();

        let next = compressed(&mut compress, &RAW);
        assert!(cursors.apply((0, 0), (2, 1), 1001, &next).unwrap().is_some());
    }

    #[test]
    fn oldest_shape_is_evicted_past_the_cache_size() {
        let mut compress = Compress::new(Compression::default(), true);
        let mut cursors = Cursors::default();

        for id in 0..65 {
            cursors
                .apply((0, 0), (2, 1), id, &compressed(&mut compress, &RAW))
                .unwrap();
        }

        assert_eq!(cursors.apply((0, 0), (0, 0), 0, &[]).unwrap(), None);
        assert!(cursors.apply((0, 0), (0, 0), 64, &[]).unwrap().is_some());
    }

    #[test]
    fn oversized_cursor_is_refused_before_inflating() {
        assert!(Cursors::default().apply((0, 0), (513, 1), 1, &[0x78, 0x9c]).is_err());
    }
}
