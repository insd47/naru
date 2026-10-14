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

impl VideoCodec {
    /// Number of horizontal bands the offer requests, each sent on its own SSRC.
    ///
    /// - HEVC takes four, like Apple's client: an LTR acknowledgment names a picture by its DONL, which only four bands carry.
    /// - H.264 takes one, since bands that reference each other smear in an ordinary decoder.
    pub fn bands(self) -> u8 {
        match self {
            Self::H264 => 1,
            Self::Hevc => 4,
        }
    }
}

/// Configuration for creating a decoder.
///
/// - H.264 takes it from the config packet's `avcC`; HEVC from the parameter sets that arrive in-stream with the keyframe.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DecoderConfig {
    pub codec: VideoCodec,
    /// Number of horizontal bands each screen comes in. Every frame is a picture of one band, numbered from the top.
    pub bands: u8,
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
    /// Band the picture covers, counted from the top; always 0 for H.264
    ///
    /// - Each HEVC band is a picture of its own, a quarter of the height rounded up to a multiple of 16, and the bands stack
    ///   from the top. Bands that did not change are not sent, so the picture last decoded for a band stays on screen.
    pub band: u8,
    /// RTP timestamp (90 kHz)
    pub timestamp: u32,
    pub keyframe: bool,
    /// NAL units without start codes
    pub nals: Vec<Vec<u8>>,
}

impl AccessUnit {
    /// Rebuilds an access unit from its RTP payloads in sequence order, with its DONL when the codec carries one.
    /// Returns `None` if a fragment is missing or malformed.
    pub fn depacketize<'a>(
        codec: VideoCodec,
        ssrc: u32,
        band: u8,
        timestamp: u32,
        payloads: impl IntoIterator<Item = &'a [u8]>,
    ) -> Option<(Self, Option<u16>)> {
        let (nals, keyframe, donl) = match codec {
            VideoCodec::H264 => {
                let nals = h264::depacketize(payloads)?;
                let keyframe = h264::is_keyframe(&nals);
                (nals, keyframe, None)
            }
            VideoCodec::Hevc => {
                let (nals, donl) = hevc::depacketize(payloads)?;
                let keyframe = hevc::is_keyframe(&nals);
                (nals, keyframe, Some(donl))
            }
        };

        let unit = Self {
            codec,
            ssrc,
            band,
            timestamp,
            keyframe,
            nals,
        };

        Some((unit, donl))
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
