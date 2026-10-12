use crate::error::{Error, Result};

// 서버는 이보다 큰 픽셀 크기를 받으면 임의의 크기로 바꿉니다.
const MAX_PIXELS: (u32, u32) = (3840, 2160);

const NAME_BYTES: usize = 120;
const DESCRIPTOR_BYTES: usize = 0x9c;
const MODE_BYTES: usize = 28;

// 동적 해상도 플래그와 가상 디스플레이 종류가 있어야 세션 중의 크기 변경 요청으로 받아들여집니다.
const DYNAMIC: u32 = 1;
const VIRTUAL: u32 = 4;
const ROTATION: u32 = 7;

/// 가상 디스플레이 설정(SetDisplayConfiguration)입니다.
///
/// - 이 설정을 보내면 Mac의 실제 화면이 가려지고, 서버는 이 크기의 가상 디스플레이를 스트리밍합니다.
/// - 세션 중에 다시 보내면 해상도 변경 요청이 됩니다. 실제로 적용된 크기는 `Layout`으로 확인합니다.
pub struct DisplayConfig {
    /// Mac의 디스플레이 목록에 보이는 이름. 119바이트를 넘으면 자릅니다.
    pub name: String,
    /// 물리 크기(mm). 서버가 DPI를 계산하는 데 씁니다.
    pub physical_mm: (f32, f32),
    /// 모드 표. 첫 모드를 현재 모드와 선호 모드로 씁니다.
    pub modes: Vec<Mode>,
}

/// 디스플레이 모드 하나입니다.
#[derive(Clone, Copy)]
pub struct Mode {
    pub pixels: (u32, u32),
    pub logical: (u32, u32),
    pub refresh_hz: f64,
    pub hdr: bool,
}

impl DisplayConfig {
    /// 메시지를 만듭니다. 모드가 없거나 픽셀 크기가 서버의 한도를 넘으면 실패합니다.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let (max_width, max_height) = MAX_PIXELS;
        let fits = |mode: &Mode| mode.pixels.0 <= max_width && mode.pixels.1 <= max_height;

        if self.modes.is_empty() || !self.modes.iter().all(fits) || self.modes.len() > 64 {
            return Err(Error::Invalid("디스플레이 설정"));
        }

        let descriptor_len = DESCRIPTOR_BYTES + MODE_BYTES * self.modes.len();

        let mut message = vec![0x1d, 0];
        message.extend_from_slice(&((8 + descriptor_len) as u16).to_be_bytes());
        message.extend_from_slice(&1u16.to_be_bytes());
        message.extend_from_slice(&1u16.to_be_bytes());
        message.extend_from_slice(&0u32.to_be_bytes());

        let mut name = [0; NAME_BYTES];
        let bytes = self.name.as_bytes();
        let len = bytes.len().min(NAME_BYTES - 1);
        name[..len].copy_from_slice(&bytes[..len]);

        message.extend_from_slice(&(descriptor_len as u16).to_be_bytes());
        message.extend_from_slice(&name);
        message.extend_from_slice(&DYNAMIC.to_be_bytes());
        message.extend_from_slice(&VIRTUAL.to_be_bytes());
        message.extend_from_slice(&self.physical_mm.0.to_be_bytes());
        message.extend_from_slice(&self.physical_mm.1.to_be_bytes());
        message.extend_from_slice(&max_width.to_be_bytes());
        message.extend_from_slice(&max_height.to_be_bytes());
        message.extend_from_slice(&0u16.to_be_bytes());
        message.extend_from_slice(&0u16.to_be_bytes());
        message.extend_from_slice(&ROTATION.to_be_bytes());
        message.extend_from_slice(&(self.modes.len() as u16).to_be_bytes());

        for mode in &self.modes {
            message.extend_from_slice(&mode.pixels.0.to_be_bytes());
            message.extend_from_slice(&mode.pixels.1.to_be_bytes());
            message.extend_from_slice(&mode.logical.0.to_be_bytes());
            message.extend_from_slice(&mode.logical.1.to_be_bytes());
            message.extend_from_slice(&mode.refresh_hz.to_be_bytes());
            message.extend_from_slice(&u32::from(mode.hdr).to_be_bytes());
        }

        Ok(message)
    }
}

// 이 앱의 가상 디스플레이입니다. 이름과 실제 크기는 Mac의 디스플레이 설정 화면에만 쓰입니다.
impl From<Mode> for DisplayConfig {
    fn from(mode: Mode) -> Self {
        Self {
            name: "Naru".into(),
            physical_mm: (369.45, 207.82),
            modes: vec![mode],
        }
    }
}

impl Mode {
    /// 논리 크기와 배율로 60Hz SDR 모드를 만듭니다. 배율 2는 Retina입니다.
    pub fn new(logical: (u32, u32), scale: f64) -> Self {
        let scaled = |value: u32| (f64::from(value) * scale).round().clamp(1.0, f64::from(u32::MAX)) as u32;

        Self {
            pixels: (scaled(logical.0), scaled(logical.1)),
            logical,
            refresh_hz: 60.0,
            hdr: false,
        }
    }

    /// 서버가 받아 주는 모드를 만듭니다. 세로 크기는 가로로 돌리고, 픽셀 크기가 한도(3840×2160)를 넘으면 비율을 유지한 채 줄입니다.
    ///
    /// - 서버는 세로 크기를 받지 않고 그 가상 디스플레이의 이전 크기로 되돌립니다. 한도 안의 540×960@2도 그랬습니다(2026-10-12 macOS 27 실측).
    ///   그래서 세로로 세운 태블릿은 가로로 돌렸을 때의 크기를 요청하고, 화면에는 위아래 여백을 두고 보입니다.
    pub fn fitting(logical: (u32, u32), scale: f64) -> Self {
        let logical = (logical.0.max(logical.1), logical.0.min(logical.1));
        let (max_width, max_height) = MAX_PIXELS;
        let ratio = (f64::from(max_width) / (f64::from(logical.0) * scale))
            .min(f64::from(max_height) / (f64::from(logical.1) * scale))
            .min(1.0);
        let shrink = |value: u32| ((f64::from(value) * ratio).floor() as u32).max(1);

        Self::new((shrink(logical.0), shrink(logical.1)), scale)
    }
}

#[cfg(test)]
mod tests {
    use super::{DisplayConfig, Mode};

    fn config(modes: Vec<Mode>) -> DisplayConfig {
        DisplayConfig {
            name: "Naru".into(),
            physical_mm: (369.45, 207.82),
            modes,
        }
    }

    #[test]
    fn five_modes_make_a_308_byte_message() {
        let message = config(vec![Mode::new((1280, 800), 2.0); 5]).encode().unwrap();

        assert_eq!(message.len(), 308);
        assert_eq!(&message[..12], [0x1d, 0, 0x01, 0x30, 0, 1, 0, 1, 0, 0, 0, 0]);
        assert_eq!(&message[12..14], 296u16.to_be_bytes());
        assert_eq!(&message[12 + 0x7a..12 + 0x82], [0, 0, 0, 1, 0, 0, 0, 4]);
        assert_eq!(&message[12 + 0x96..12 + 0x9c], [0, 0, 0, 7, 0, 5]);

        let mode = &message[12 + 0x9c..12 + 0x9c + 28];
        assert_eq!(
            &mode[..16],
            [0, 0, 0x0a, 0, 0, 0, 0x06, 0x40, 0, 0, 0x05, 0, 0, 0, 0x03, 0x20]
        );
        assert_eq!(&mode[16..24], 60f64.to_be_bytes());
    }

    #[test]
    fn pixels_beyond_the_server_limit_are_refused() {
        assert!(config(vec![Mode::new((2000, 1200), 2.0)]).encode().is_err());
        assert!(config(Vec::new()).encode().is_err());
    }

    // 세로로 세운 iPad Pro 11형(834×1210@2)이 높이 한도를 넘어 접속에 실패했습니다(2026-10-12 TestFlight).
    #[test]
    fn fitting_turns_portrait_into_landscape_within_the_limit() {
        let portrait = Mode::fitting((834, 1210), 2.0);
        assert_eq!((portrait.logical, portrait.pixels), ((1210, 834), (2420, 1668)));
        assert!(config(vec![portrait]).encode().is_ok());

        assert_eq!(Mode::fitting((2000, 1200), 2.0).pixels, (3600, 2160));
    }
}
