use std::fmt;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("payload too short: need {need} bytes, got {got}")]
    Short { need: usize, got: usize },
    #[error("unexpected payload length {0}")]
    Length(usize),
}

fn u8_at(payload: &[u8], index: usize) -> Result<u8, DecodeError> {
    payload.get(index).copied().ok_or(DecodeError::Short {
        need: index + 1,
        got: payload.len(),
    })
}

fn u16_at(payload: &[u8], index: usize) -> Result<u16, DecodeError> {
    let bytes = payload.get(index..index + 2).ok_or(DecodeError::Short {
        need: index + 2,
        got: payload.len(),
    })?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Firmware {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl Firmware {
    pub fn parse(payload: &[u8]) -> Result<Self, DecodeError> {
        Ok(Self {
            major: u16_at(payload, 0)?,
            minor: u16_at(payload, 2)?,
            patch: u16_at(payload, 4)?,
        })
    }
}

impl fmt::Display for Firmware {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AncModes {
    pub anti_wind: u8,
    pub comfort: u8,
    pub adaptive: u8,
}

impl AncModes {
    pub fn parse(payload: &[u8]) -> Result<Self, DecodeError> {
        if payload.len() < 6 || !payload.len().is_multiple_of(2) {
            return Err(DecodeError::Length(payload.len()));
        }
        let mut modes = Self {
            anti_wind: 0,
            comfort: 0,
            adaptive: 0,
        };
        for pair in payload.chunks_exact(2) {
            match pair[0] {
                1 => modes.anti_wind = pair[1],
                2 => modes.comfort = pair[1],
                3 => modes.adaptive = pair[1],
                _ => {}
            }
        }
        Ok(modes)
    }

    pub fn encode(&self, mode: u8, state: u8) -> [u8; 2] {
        [mode, state]
    }
}

impl fmt::Display for AncModes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "adaptive={} anti-wind={} comfort={}",
            self.adaptive, self.anti_wind, self.comfort
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqConfig {
    pub bands: u8,
    pub min_db: f32,
    pub max_db: f32,
    pub preset: u8,
    pub user_presets: u8,
}

impl EqConfig {
    pub fn parse(payload: &[u8]) -> Result<Self, DecodeError> {
        Ok(Self {
            bands: u8_at(payload, 0)?,
            min_db: f32::from(u8_at(payload, 1)? as i8) / 10.0,
            max_db: f32::from(u8_at(payload, 2)? as i8) / 10.0,
            preset: u8_at(payload, 3)?,
            user_presets: u8_at(payload, 4)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SliderGain(pub i8);

impl SliderGain {
    pub fn parse(payload: &[u8]) -> Result<Self, DecodeError> {
        Ok(Self(u8_at(payload, 0)? as i8))
    }

    pub fn db(self) -> f32 {
        f32::from(self.0) / 10.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Sbc,
    Aac,
    Aptx,
    AptxLl,
    Mp3,
    AptxHd,
    FastStream,
    Lhdc,
    AptxAdaptive,
    AptxLossless,
    Lc3,
    None,
    Unknown(u8),
}

impl Codec {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Sbc,
            1 => Self::Aac,
            2 => Self::Aptx,
            3 => Self::AptxLl,
            4 => Self::Mp3,
            5 => Self::AptxHd,
            6 => Self::FastStream,
            7 => Self::Lhdc,
            8 => Self::AptxAdaptive,
            9 => Self::AptxLossless,
            10 => Self::Lc3,
            255 => Self::None,
            other => Self::Unknown(other),
        }
    }
}

impl fmt::Display for Codec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Sbc => "SBC",
            Self::Aac => "AAC",
            Self::Aptx => "aptX",
            Self::AptxLl => "aptX LL",
            Self::Mp3 => "MP3",
            Self::AptxHd => "aptX HD",
            Self::FastStream => "FastStream",
            Self::Lhdc => "LHDC",
            Self::AptxAdaptive => "aptX Adaptive",
            Self::AptxLossless => "aptX Lossless",
            Self::Lc3 => "LC3",
            Self::None => "none",
            Self::Unknown(value) => return write!(f, "unknown({value})"),
        };
        f.write_str(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Charging {
    pub states: Vec<u8>,
}

impl Charging {
    pub fn parse(payload: &[u8]) -> Result<Self, DecodeError> {
        if payload.is_empty() {
            return Err(DecodeError::Length(0));
        }
        Ok(Self {
            states: payload.to_vec(),
        })
    }

    pub fn is_charging(&self) -> bool {
        self.states.contains(&1)
    }

    pub fn label(&self) -> &'static str {
        if self.is_charging() {
            "charging"
        } else if self.states.contains(&2) {
            "charged"
        } else {
            "discharging"
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedDevice {
    pub index: u8,
    pub priority: u8,
    pub connected: bool,
    pub name: String,
}

impl PairedDevice {
    pub fn parse(payload: &[u8]) -> Result<Self, DecodeError> {
        if payload.len() < 3 {
            return Err(DecodeError::Short {
                need: 3,
                got: payload.len(),
            });
        }
        let raw = &payload[3..];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        Ok(Self {
            index: payload[0],
            priority: payload[1],
            connected: payload[2] != 0,
            name: String::from_utf8_lossy(&raw[..end]).into_owned(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalState {
    Unknown,
    InCase,
    OffHead,
    OnHead,
    Other(u8),
}

impl PhysicalState {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Unknown,
            1 => Self::InCase,
            2 => Self::OffHead,
            3 => Self::OnHead,
            other => Self::Other(other),
        }
    }
}

impl fmt::Display for PhysicalState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("unknown"),
            Self::InCase => f.write_str("in case"),
            Self::OffHead => f.write_str("not on head"),
            Self::OnHead => f.write_str("on head"),
            Self::Other(value) => write!(f, "unknown({value})"),
        }
    }
}

pub fn audio_mode_name(mode: u16) -> &'static str {
    match mode {
        0 => "off (headset tuning)",
        1 => "user EQ",
        2 => "podcast",
        3 => "personalized sound",
        4 => "parametric EQ",
        5 => "hearing enhancement",
        _ => "unknown",
    }
}

pub fn prompt_mode_name(mode: u8) -> &'static str {
    match mode {
        0 => "off",
        1 => "tones only",
        2 => "tones + voice",
        _ => "unknown",
    }
}

pub fn prompt_language_name(language: u8) -> &'static str {
    match language {
        0 => "EN",
        1 => "DE",
        2 => "FR",
        3 => "ES",
        4 => "ZH",
        5 => "JA",
        6 => "RU",
        7 => "KO",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_firmware_and_model() {
        let firmware = Firmware::parse(&[0, 3, 0, 38, 0, 3]).unwrap();
        assert_eq!(firmware.to_string(), "3.38.3");
        let device = PairedDevice::parse(&[1, 0, 1, b'a', b'm', b'o', 0]).unwrap();
        assert_eq!(device.name, "amo");
        assert!(device.connected);
    }

    #[test]
    fn parses_anc_pairs_and_eq_config() {
        let modes = AncModes::parse(&[1, 0, 2, 0, 3, 1]).unwrap();
        assert_eq!(
            modes,
            AncModes {
                anti_wind: 0,
                comfort: 0,
                adaptive: 1
            }
        );
        let eq = EqConfig::parse(&[5, 0xc4, 0x3c, 0, 0]).unwrap();
        assert_eq!((eq.bands, eq.min_db, eq.max_db), (5, -6.0, 6.0));
        assert_eq!(SliderGain::parse(&[0xec]).unwrap().db(), -2.0);
    }

    #[test]
    fn maps_codecs_and_charging() {
        assert_eq!(Codec::from_u8(5).to_string(), "aptX HD");
        assert_eq!(Codec::from_u8(255).to_string(), "none");
        assert_eq!(Charging::parse(&[0, 0]).unwrap().label(), "discharging");
        assert_eq!(Charging::parse(&[1, 0]).unwrap().label(), "charging");
        assert_eq!(PhysicalState::from_u8(3).to_string(), "on head");
    }
}
