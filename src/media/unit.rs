//! 두 코덱이 함께 쓰는 접근 단위와 디코더 설정입니다.
use super::{h264, hevc};

const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// 서버가 보내는 영상의 코덱입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VideoCodec {
    /// H.264 4:2:0
    H264,
    /// HEVC Range Extensions 4:4:4 8비트
    Hevc,
}

/// 디코더를 만들 설정입니다.
///
/// - H.264는 설정 패킷의 `avcC`에서, HEVC는 키프레임과 함께 스트림 안으로 오는 파라미터 세트에서 꺼냅니다.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DecoderConfig {
    pub codec: VideoCodec,
    pub profile: u8,
    /// H.264의 profile_compatibility. HEVC는 0입니다.
    pub compatibility: u8,
    pub level: u8,
    /// 시작 코드 없는 VPS NAL 목록. H.264는 비어 있습니다.
    pub vps: Vec<Vec<u8>>,
    /// 시작 코드 없는 SPS NAL 목록
    pub sps: Vec<Vec<u8>>,
    /// 시작 코드 없는 PPS NAL 목록
    pub pps: Vec<Vec<u8>>,
}

impl DecoderConfig {
    /// VPS, SPS, PPS를 Annex-B(시작 코드) 형식으로 이어 붙입니다.
    pub fn annex_b(&self) -> Vec<u8> {
        annex_b(self.vps.iter().chain(&self.sps).chain(&self.pps))
    }
}

/// 디코더에 넣을 수 있는 완성된 접근 단위입니다.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AccessUnit {
    pub codec: VideoCodec,
    pub ssrc: u32,
    /// RTP 타임스탬프(90 kHz)
    pub timestamp: u32,
    pub keyframe: bool,
    /// 시작 코드 없는 NAL 목록
    pub nals: Vec<Vec<u8>>,
}

impl AccessUnit {
    /// 한 접근 단위의 RTP 페이로드들을 순번 순서로 받아 되돌립니다. 조각이 빠졌거나 형식이 맞지 않으면 `None`입니다.
    pub fn depacketize<'a>(
        codec: VideoCodec,
        ssrc: u32,
        timestamp: u32,
        payloads: impl IntoIterator<Item = &'a [u8]>,
    ) -> Option<Self> {
        let (nals, keyframe) = match codec {
            VideoCodec::H264 => {
                let nals = h264::depacketize(payloads)?;
                let keyframe = h264::is_keyframe(&nals);
                (nals, keyframe)
            }
            VideoCodec::Hevc => {
                let nals = hevc::depacketize(payloads)?;
                let keyframe = hevc::is_keyframe(&nals);
                (nals, keyframe)
            }
        };

        Some(Self {
            codec,
            ssrc,
            timestamp,
            keyframe,
            nals,
        })
    }

    /// SPS NAL이 들어 있는지 확인합니다. 접근 단위에 SPS가 이미 있으면 설정을 따로 붙이지 않습니다.
    pub fn has_sps(&self) -> bool {
        self.nals.iter().any(|nal| match self.codec {
            VideoCodec::H264 => nal.first().is_some_and(|header| header & 0x1f == h264::NAL_SPS),
            VideoCodec::Hevc => hevc::nal_type(nal) == Some(hevc::NAL_SPS),
        })
    }

    /// NAL을 Annex-B(시작 코드) 형식으로 이어 붙입니다.
    pub fn annex_b(&self) -> Vec<u8> {
        annex_b(&self.nals)
    }
}

fn annex_b<'a>(nals: impl IntoIterator<Item = &'a Vec<u8>>) -> Vec<u8> {
    nals.into_iter()
        .flat_map(|nal| START_CODE.iter().chain(nal))
        .copied()
        .collect()
}
