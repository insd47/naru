use crate::error::{Error, Result};
use flate2::{Decompress, FlushDecompress, Status};

// 서버의 zlib 데이터는 종료 블록 없이 동기 플러시로 끝납니다. 한 번에 푸는 함수는 이를 잘린 스트림으로 거부하므로,
// 스트림 해제기에 넣고 정해진 길이가 나오면 성공으로 봅니다. 출력은 `len`을 넘지 않습니다.
pub fn inflate(stream: &mut Decompress, input: &[u8], len: usize, what: &'static str) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(len);
    let start = stream.total_in();

    while output.len() < len {
        let consumed = (stream.total_in() - start) as usize;
        let before = (stream.total_in(), stream.total_out());

        let status = stream
            .decompress_vec(&input[consumed..], &mut output, FlushDecompress::Sync)
            .map_err(|_| Error::Malformed(what))?;

        if status == Status::StreamEnd || before == (stream.total_in(), stream.total_out()) {
            break;
        }
    }

    if output.len() != len {
        return Err(Error::Malformed(what));
    }

    Ok(output)
}

// zlib 머리(CMF, FLG)로 시작하는지 확인합니다. 새 스트림인지, 이전 스트림에 이어지는 데이터인지 가릴 때 씁니다.
pub fn starts_stream(data: &[u8]) -> bool {
    matches!(data, [cmf, flg, ..] if cmf & 0x0f == 8 && (u16::from(*cmf) << 8 | u16::from(*flg)) % 31 == 0)
}

#[cfg(test)]
mod tests {
    use super::{inflate, starts_stream};
    use flate2::{Compress, Compression, Decompress, FlushCompress};

    // 서버처럼 동기 플러시로 끝나는 스트림을 만듭니다.
    fn sync_flushed(compress: &mut Compress, data: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(data.len() + 64);
        compress.compress_vec(data, &mut output, FlushCompress::Sync).unwrap();
        output
    }

    #[test]
    fn sync_flushed_chunks_of_one_stream_inflate_in_turn() {
        let mut compress = Compress::new(Compression::default(), true);
        let first = sync_flushed(&mut compress, &[1; 300]);
        let second = sync_flushed(&mut compress, &[2; 200]);

        assert!(starts_stream(&first));

        let mut stream = Decompress::new(true);
        assert_eq!(inflate(&mut stream, &first, 300, "test").unwrap(), [1; 300]);
        assert_eq!(inflate(&mut stream, &second, 200, "test").unwrap(), [2; 200]);
    }

    #[test]
    fn output_shorter_than_declared_is_malformed() {
        let mut compress = Compress::new(Compression::default(), true);
        let data = sync_flushed(&mut compress, &[7; 1000]);

        let mut stream = Decompress::new(true);
        assert!(inflate(&mut stream, &data, 1001, "test").is_err());
    }
}
