//! Rectangle encoding numbers for framebuffer updates. The client announces the ones it supports, and the server tags each rectangle with one.

pub const ZLIB: i32 = 6;
pub const ZRLE: i32 = 16;
pub const DESKTOP_SIZE: i32 = -223;

pub const MEDIA_STREAM: i32 = 0x3f2;
pub const STILL_IMAGE: i32 = 0x3f3;
pub const STILL_IMAGE_LEGACY: i32 = 0x3ea;
pub const UNNAMED_44C: i32 = 0x44c;
pub const UNNAMED_44D: i32 = 0x44d;
pub const KEY_EXCHANGE: i32 = 0x44f;
pub const CURSOR: i32 = 0x450;
pub const DISPLAY_LAYOUT: i32 = 0x451;
pub const VENDOR_KEYSYMS: i32 = 0x453;
pub const INPUT_SOURCE: i32 = 0x455;
pub const DEVICE_INFO: i32 = 0x456;

/// Encodings the client announces. The order is passed on to the server too.
pub const SUPPORTED: [i32; 13] = [
    MEDIA_STREAM,
    STILL_IMAGE,
    STILL_IMAGE_LEGACY,
    ZLIB,
    ZRLE,
    CURSOR,
    UNNAMED_44C,
    DESKTOP_SIZE,
    UNNAMED_44D,
    DISPLAY_LAYOUT,
    VENDOR_KEYSYMS,
    INPUT_SOURCE,
    DEVICE_INFO,
];
