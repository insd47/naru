// The offer's MediaBlob protobuf format. Fields whose meaning is unconfirmed are named by number.
//
// Scalar fields are declared as `Option` so they are encoded explicitly, letting zero values reach the server.

#[derive(Clone, PartialEq, prost::Message)]
pub struct MediaBlob {
    #[prost(uint32, optional, tag = "1")]
    pub field_1: Option<u32>,
    #[prost(uint32, optional, tag = "2")]
    pub field_2: Option<u32>,
    #[prost(message, optional, tag = "3")]
    pub audio: Option<AudioStream>,
    #[prost(message, optional, tag = "5")]
    pub video: Option<VideoStream>,
    #[prost(string, optional, tag = "6")]
    pub client_name: Option<String>,
    #[prost(uint32, optional, tag = "8")]
    pub field_8: Option<u32>,
    #[prost(message, repeated, tag = "9")]
    pub audio_tiers: Vec<AudioTier>,
    #[prost(uint64, optional, tag = "13")]
    pub timestamp_ns: Option<u64>,
    #[prost(uint32, optional, tag = "14")]
    pub field_14: Option<u32>,
    #[prost(uint32, optional, tag = "16")]
    pub field_16: Option<u32>,
    #[prost(uint32, optional, tag = "18")]
    pub field_18: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct VideoStream {
    #[prost(uint32, optional, tag = "1")]
    pub ssrc: Option<u32>,
    #[prost(uint32, optional, tag = "2")]
    pub feedback: Option<u32>,
    #[prost(message, repeated, tag = "3")]
    pub banks: Vec<CodecBank>,
    #[prost(uint32, optional, tag = "6")]
    pub tiles: Option<u32>,
    #[prost(uint32, optional, tag = "7")]
    pub ltr: Option<u32>,
    #[prost(uint32, optional, tag = "8")]
    pub field_8: Option<u32>,
    #[prost(uint32, optional, tag = "9")]
    pub field_9: Option<u32>,
    #[prost(uint32, optional, tag = "12")]
    pub field_12: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CodecBank {
    #[prost(uint32, optional, tag = "1")]
    pub id: Option<u32>,
    #[prost(message, repeated, tag = "2")]
    pub resolutions: Vec<Resolution>,
    #[prost(string, optional, tag = "3")]
    pub features: Option<String>,
    #[prost(uint32, optional, tag = "4")]
    pub field_4: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Resolution {
    #[prost(uint32, optional, tag = "1")]
    pub field_1: Option<u32>,
    #[prost(uint32, optional, tag = "2")]
    pub kind: Option<u32>,
    #[prost(uint32, optional, tag = "3")]
    pub field_3: Option<u32>,
    #[prost(uint32, optional, tag = "4")]
    pub field_4: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct AudioStream {
    #[prost(uint32, optional, tag = "1")]
    pub ssrc: Option<u32>,
    #[prost(uint32, optional, tag = "2")]
    pub field_2: Option<u32>,
    #[prost(uint32, optional, tag = "3")]
    pub field_3: Option<u32>,
    #[prost(uint32, optional, tag = "4")]
    pub bitrate: Option<u32>,
    #[prost(uint32, optional, tag = "5")]
    pub field_5: Option<u32>,
    #[prost(uint32, optional, tag = "6")]
    pub field_6: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct AudioTier {
    #[prost(uint32, optional, tag = "1")]
    pub field_1: Option<u32>,
    #[prost(uint32, optional, tag = "2")]
    pub field_2: Option<u32>,
    #[prost(uint32, optional, tag = "3")]
    pub field_3: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct EndpointInfo {
    #[prost(uint32, optional, tag = "1")]
    pub field_1: Option<u32>,
    #[prost(uint32, optional, tag = "2")]
    pub field_2: Option<u32>,
    #[prost(string, optional, tag = "3")]
    pub model: Option<String>,
    #[prost(string, optional, tag = "4")]
    pub version: Option<String>,
    #[prost(string, optional, tag = "5")]
    pub build: Option<String>,
}

// Declares only the fields read from the server's answer; prost skips the rest.
#[derive(Clone, PartialEq, prost::Message)]
pub struct AnswerBlob {
    #[prost(message, optional, tag = "5")]
    pub video: Option<VideoAnswer>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct VideoAnswer {
    #[prost(uint32, optional, tag = "4")]
    pub width: Option<u32>,
    #[prost(uint32, optional, tag = "5")]
    pub height: Option<u32>,
}
