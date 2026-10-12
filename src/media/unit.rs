//! Access units and decoder configs shared by both codecs.
use super::{h264, hevc};

const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// Codec of the video the server sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VideoCodec {
    /// H.264 4:2:0
    H264,
    /// HEVC Range Extensions, 4:4:4 8-bit
    Hevc,
}

/// Configuration for creating a decoder.
///
/// - H.264 takes it from the config packet's `avcC`; HEVC from the parameter sets that arrive in-stream with the keyframe.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DecoderConfig {
    pub codec: VideoCodec,
    pub profile: u8,
    /// H.264 profile_compatibility; 0 for HEVC
    pub compatibility: u8,
    pub level: u8,
    /// VPS NAL units without start codes; empty for H.264
    pub vps: Vec<Vec<u8>>,
    /// SPS NAL units without start codes
    pub sps: Vec<Vec<u8>>,
    /// PPS NAL units without start codes
    pub pps: Vec<Vec<u8>>,
}

impl DecoderConfig {
    /// Concatenates VPS, SPS, and PPS in Annex B (start code) format.
    pub fn annex_b(&self) -> Vec<u8> {
        annex_b(self.vps.iter().chain(&self.sps).chain(&self.pps))
    }
}

/// A complete access unit ready for the decoder.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AccessUnit {
    pub codec: VideoCodec,
    pub ssrc: u32,
    /// RTP timestamp (90 kHz)
    pub timestamp: u32,
    pub keyframe: bool,
    /// NAL units without start codes
    pub nals: Vec<Vec<u8>>,
}

impl AccessUnit {
    /// Rebuilds an access unit from its RTP payloads in sequence order. Returns `None` if a fragment is missing or malformed.
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

    /// Checks for an SPS NAL unit. If the access unit already has one, the config isn't prepended separately.
    pub fn has_sps(&self) -> bool {
        self.nals.iter().any(|nal| match self.codec {
            VideoCodec::H264 => nal.first().is_some_and(|header| header & 0x1f == h264::NAL_SPS),
            VideoCodec::Hevc => hevc::nal_type(nal) == Some(hevc::NAL_SPS),
        })
    }

    /// Concatenates the NAL units in Annex B (start code) format.
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
