use super::zlib;
use crate::error::{Error, Result};
use flate2::Decompress;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

// Server cache ids only mean something within a session. The oldest shapes are evicted first.
const CACHE_SIZE: usize = 64;
const MAX_SIDE: u16 = 512;

/// A cursor shape.
#[derive(Debug, PartialEq)]
pub struct CursorImage {
    /// Point in the image that corresponds to the pointer position, in pixels
    pub hotspot: (u16, u16),
    pub width: u16,
    pub height: u16,
    /// Row-major RGBA pixels. Alpha is not premultiplied.
    pub rgba: Vec<u8>,
}

/// Parses cursor rectangles and stores shapes under the server's cache ids.
///
/// - A rectangle with compressed data stores its shape and switches to it; one without data switches to a stored shape.
pub struct Cursors {
    shapes: HashMap<u32, Arc<CursorImage>>,
    order: VecDeque<u32>,
    // Kept across rectangles because data may continue the previous rectangle's zlib stream.
    stream: Decompress,
}

impl Cursors {
    /// Applies one cursor rectangle and returns the resulting shape.
    ///
    /// - The server occasionally selects an id that was never stored. That returns `None`, and the previous shape should stay.
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
            return Err(Error::Malformed("cursor size"));
        }

        if zlib::starts_stream(data) {
            self.stream = Decompress::new(true);
        }

        // A separate alpha plane follows the BGRA pixel plane, and alpha is taken from that trailing plane.
        let pixels = usize::from(width) * usize::from(height);
        let raw = zlib::inflate(&mut self.stream, data, pixels * 5, "cursor image")?;
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
            let oldest = self.order.pop_front().expect("length was checked");
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
    use flate2::{Compress, Compression, FlushCompress};

    fn compressed(compress: &mut Compress, raw: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(raw.len() + 64);
        compress.compress_vec(raw, &mut output, FlushCompress::Sync).unwrap();
        output
    }

    // A 2×1 cursor: blue and red pixels with an alpha plane of 128, 255
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
