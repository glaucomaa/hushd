pub mod decode;
pub mod frame;
pub mod ids;

pub use decode::{
    audio_mode_name, prompt_language_name, prompt_mode_name, AncModes, Charging, Codec,
    DecodeError, EqConfig, Firmware, PairedDevice, PhysicalState, SliderGain,
};
pub use frame::{Deframer, EncodeError, Frame, Kind};
pub use ids::{cmd, feature, QUALCOMM, SENNHEISER};

/// RFCOMM service the MOMENTUM 4 publishes for GAIA.
pub const SPP_UUID: &str = "a2129ff3-081b-4c45-8afe-469d9c4842ec";
pub const SPP_UUID_BYTES: [u8; 16] = [
    0xa2, 0x12, 0x9f, 0xf3, 0x08, 0x1b, 0x4c, 0x45, 0x8a, 0xfe, 0x46, 0x9d, 0x9c, 0x48, 0x42, 0xec,
];

/// Fallback for firmware 3.38.3 when SDP is unavailable.
pub const SPP_CHANNELS: [u8; 2] = [1, 15];

impl Frame {
    pub fn sen(command: u16, payload: impl Into<Vec<u8>>) -> Self {
        Self::new(SENNHEISER, command, payload)
    }

    pub fn qualcomm(command: u16, payload: impl Into<Vec<u8>>) -> Self {
        Self::new(QUALCOMM, command, payload)
    }
}
