use std::collections::VecDeque;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use gaia::{
    cmd, AncModes, Charging, Codec, DecodeError, Deframer, EqConfig, Firmware, Frame, Kind,
    PairedDevice, SliderGain,
};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

/// Parses "AA:BB:CC:DD:EE:FF" into bytes, independent of the platform backend.
pub fn parse_address(text: &str) -> Result<[u8; 6], String> {
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() != 6 {
        return Err(format!("invalid bluetooth address: {text}"));
    }
    let mut address = [0u8; 6];
    for (slot, part) in address.iter_mut().zip(parts) {
        *slot = u8::from_str_radix(part, 16)
            .map_err(|_| format!("invalid bluetooth address: {text}"))?;
    }
    Ok(address)
}

/// Minimal transport the session needs: a byte stream with read timeouts.
/// Linux uses RFCOMM sockets, macOS will use IOBluetooth's RFCOMM channel.
pub trait Transport: Send {
    fn send(&self, bytes: &[u8]) -> std::io::Result<()>;
    fn recv(&self, buffer: &mut [u8], timeout: Duration) -> std::io::Result<Option<usize>>;
}

#[cfg(target_os = "linux")]
impl Transport for rfcomm::Link {
    fn send(&self, bytes: &[u8]) -> std::io::Result<()> {
        rfcomm::Link::send(self, bytes)
    }

    fn recv(&self, buffer: &mut [u8], timeout: Duration) -> std::io::Result<Option<usize>> {
        rfcomm::Link::recv(self, buffer, timeout)
    }
}

/// Error code the headset returns when a write would not change anything.
pub const NO_CHANGE: u8 = 0x80;

/// Sennheiser's shipped curves, in dB per band (63/250/1k/4k/8k). The headset
/// stores only the resulting gains, so presets stay client-side data.
pub struct Preset {
    pub name: &'static str,
    pub gains_db: [f32; 5],
}

pub const PRESETS: [Preset; 7] = [
    Preset {
        name: "rock",
        gains_db: [0.0, 2.0, 2.5, 1.5, -2.0],
    },
    Preset {
        name: "pop",
        gains_db: [0.0, -2.5, 0.0, 2.5, 0.0],
    },
    Preset {
        name: "dance",
        gains_db: [3.5, 2.0, -1.5, 1.5, 3.0],
    },
    Preset {
        name: "hip-hop",
        gains_db: [3.0, 1.5, -1.5, 0.0, -1.5],
    },
    Preset {
        name: "classical",
        gains_db: [-2.0, -1.5, 0.0, 3.5, 4.0],
    },
    Preset {
        name: "movie",
        gains_db: [0.0, 0.0, 2.0, 2.0, -2.0],
    },
    Preset {
        name: "jazz",
        gains_db: [-3.2, 0.0, 2.2, 2.2, 0.0],
    },
];

/// Which shipped curve the headset currently holds, if any.
pub fn preset_for_gains(gains_db: &[f32]) -> Option<&'static str> {
    if gains_db.len() != 5 {
        return None;
    }
    PRESETS
        .iter()
        .find(|preset| {
            preset
                .gains_db
                .iter()
                .zip(gains_db)
                .all(|(want, got)| (want - got).abs() < 0.06)
        })
        .map(|preset| preset.name)
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NowPlaying {
    pub player: String,
    pub artist: Option<String>,
    pub title: Option<String>,
    pub playing: bool,
}

impl NowPlaying {
    pub fn label(&self) -> String {
        match (&self.artist, &self.title) {
            (Some(artist), Some(title)) => format!("{artist} — {title}"),
            (None, Some(title)) => title.clone(),
            (Some(artist), None) => artist.clone(),
            (None, None) => self.player.clone(),
        }
    }
}

pub fn preset(name: &str) -> Option<&'static Preset> {
    let wanted = name.trim().to_ascii_lowercase();
    PRESETS.iter().find(|preset| preset.name == wanted)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Rfcomm(#[from] rfcomm::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("link closed while waiting for {0:#06x}")]
    Closed(u16),
    #[error("no response to {0:#06x}")]
    Timeout(u16),
    #[error("device rejected {command:#06x} with code {code}")]
    Device { command: u16, code: u8 },
    #[error(transparent)]
    Encode(#[from] gaia::EncodeError),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("reply {0:#06x} does not belong to the request")]
    Unexpected(u16),
    #[error("link closed")]
    LinkClosed,
    #[error("unknown property {0}")]
    UnknownProperty(String),
    #[error("this platform has no GAIA transport yet: {0}")]
    Unsupported(&'static str),
}

pub struct Session {
    transport: Box<dyn Transport>,
    deframer: Deframer,
    inbox: VecDeque<Frame>,
    timeout: Duration,
    pub channel: u8,
    pub address: Option<String>,
}

impl Session {
    pub fn from_transport(transport: Box<dyn Transport>, channel: u8) -> Self {
        Self {
            transport,
            deframer: Deframer::new(),
            inbox: VecDeque::new(),
            timeout: DEFAULT_TIMEOUT,
            channel,
            address: None,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn open(address: [u8; 6], channels: &[u8], timeout: Duration) -> Result<Self, Error> {
        let (link, channel) = rfcomm::connect_any(address, channels, timeout)?;
        Ok(Self {
            transport: Box::new(link),
            address: Some(
                address
                    .iter()
                    .map(|byte| format!("{byte:02X}"))
                    .collect::<Vec<_>>()
                    .join(":"),
            ),
            deframer: Deframer::new(),
            inbox: VecDeque::new(),
            timeout,
            channel,
        })
    }

    #[cfg(target_os = "linux")]
    pub fn open_resolved(address: [u8; 6], timeout: Duration) -> Result<Self, Error> {
        let channels =
            rfcomm::sdp::resolve(address, gaia::SPP_UUID_BYTES).unwrap_or_else(|error| {
                eprintln!("m4: SDP lookup failed ({error}); trying measured channels 1 and 15");
                gaia::SPP_CHANNELS.to_vec()
            });
        Self::open(address, &channels, timeout)
    }

    pub fn request(&mut self, request: Frame) -> Result<Frame, Error> {
        let base = request.base();
        self.drop_pending_replies();
        self.transport.send(&request.encode()?)?;
        let deadline = Instant::now() + self.timeout;
        loop {
            if let Some(index) = self.inbox.iter().position(|frame| {
                frame.base() == base && matches!(frame.kind(), Kind::Response | Kind::Error)
            }) {
                let frame = self.inbox.remove(index).expect("index from position");
                // The headset occasionally duplicates a reply; drop the copies so the
                // next request cannot consume a stale answer.
                self.inbox.retain(|queued| {
                    !(queued.base() == base
                        && matches!(queued.kind(), Kind::Response | Kind::Error))
                });
                return match frame.kind() {
                    Kind::Error => Err(Error::Device {
                        command: base,
                        code: frame.payload.first().copied().unwrap_or(0),
                    }),
                    _ => Ok(frame),
                };
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(Error::Timeout(base));
            }
            let mut buffer = [0u8; 512];
            match self.transport.recv(&mut buffer, deadline - now)? {
                Some(0) => return Err(Error::Closed(base)),
                Some(count) => {
                    self.deframer.push(&buffer[..count]);
                    while let Some(frame) = self.deframer.next_frame() {
                        self.inbox.push_back(frame);
                    }
                }
                None => return Err(Error::Timeout(base)),
            }
        }
    }

    /// Reads unsolicited traffic without sending anything.
    pub fn poll(&mut self, timeout: Duration) -> Result<Vec<Frame>, Error> {
        let mut buffer = [0u8; 512];
        match self.transport.recv(&mut buffer, timeout)? {
            Some(0) => Err(Error::LinkClosed),
            Some(count) => {
                self.deframer.push(&buffer[..count]);
                while let Some(frame) = self.deframer.next_frame() {
                    self.inbox.push_back(frame);
                }
                Ok(self.take_notifications())
            }
            None => Ok(Vec::new()),
        }
    }

    /// Everything the headset pushed while we were waiting for replies.
    pub fn take_notifications(&mut self) -> Vec<Frame> {
        let (notifications, rest): (VecDeque<Frame>, VecDeque<Frame>) = self
            .inbox
            .drain(..)
            .partition(|frame| frame.kind() == Kind::Notification);
        self.inbox = rest;
        notifications.into()
    }

    pub fn subscribe(&mut self, features: &[u8]) -> Result<(), Error> {
        for &feature in features {
            self.request(Frame::sen(cmd::REGISTER_NOTIFICATION, vec![feature]))?;
        }
        Ok(())
    }

    /// Anything already waiting on the wire belongs to an earlier request (the
    /// firmware sometimes duplicates replies); only notifications may be kept.
    fn drop_pending_replies(&mut self) {
        loop {
            let mut buffer = [0u8; 512];
            match self.transport.recv(&mut buffer, Duration::ZERO) {
                Ok(Some(count)) if count > 0 => self.deframer.push(&buffer[..count]),
                _ => break,
            }
        }
        while let Some(frame) = self.deframer.next_frame() {
            self.inbox.push_back(frame);
        }
        self.inbox
            .retain(|frame| frame.kind() == Kind::Notification);
    }

    fn get(&mut self, command: u16) -> Result<Frame, Error> {
        self.request(Frame::sen(command, Vec::new()))
    }

    fn get_with(&mut self, command: u16, payload: impl Into<Vec<u8>>) -> Result<Frame, Error> {
        self.request(Frame::sen(command, payload))
    }

    pub fn firmware(&mut self) -> Result<Firmware, Error> {
        Ok(Firmware::parse(&self.get(cmd::FW_VERSION_GET)?.payload)?)
    }

    pub fn model(&mut self) -> Result<String, Error> {
        let frame = self.get(cmd::MODEL_ID_GET)?;
        Ok(string_payload(&frame.payload))
    }

    pub fn gaia_api(&mut self) -> Result<(u8, u8), Error> {
        let frame = self.request(Frame::qualcomm(cmd::CORE_API_VERSION_GET, Vec::new()))?;
        let major = *frame
            .payload
            .first()
            .ok_or(Error::Decode(DecodeError::Length(0)))?;
        let minor = *frame
            .payload
            .get(1)
            .ok_or(Error::Decode(DecodeError::Length(1)))?;
        Ok((major, minor))
    }

    pub fn serial(&mut self) -> Result<String, Error> {
        let frame = self.request(Frame::qualcomm(cmd::CORE_SERIAL_GET, Vec::new()))?;
        Ok(string_payload(&frame.payload))
    }

    pub fn hardware_revision(&mut self) -> Result<u16, Error> {
        let frame = self.get(cmd::HW_REVISION_GET)?;
        word_at(&frame, 0)
    }

    pub fn battery(&mut self) -> Result<u8, Error> {
        let frame = self.get(cmd::BATTERY_GET)?;
        frame
            .payload
            .first()
            .copied()
            .ok_or(Error::Decode(DecodeError::Length(0)))
    }

    pub fn charging(&mut self) -> Result<Charging, Error> {
        Ok(Charging::parse(&self.get(cmd::CHARGING_GET)?.payload)?)
    }

    pub fn anc_modes(&mut self) -> Result<AncModes, Error> {
        Ok(AncModes::parse(&self.get(cmd::ANC_GET)?.payload)?)
    }

    pub fn anc_status(&mut self) -> Result<u8, Error> {
        byte_at(&self.get(cmd::ANC_STATUS_GET)?, 0)
    }

    pub fn set_anc_status(&mut self, on: bool) -> Result<(), Error> {
        ignore_no_change(self.request(Frame::sen(cmd::ANC_STATUS_SET, vec![u8::from(on)])))
    }

    pub fn transparency(&mut self) -> Result<u8, Error> {
        byte_at(&self.get(cmd::TRANSPARENCY_GET)?, 0)
    }

    pub fn set_transparency(&mut self, level: u8) -> Result<(), Error> {
        ignore_no_change(self.request(Frame::sen(cmd::TRANSPARENCY_SET, vec![level.min(100)])))
    }

    pub fn set_anc_mode(&mut self, mode: u8, state: u8) -> Result<(), Error> {
        ignore_no_change(self.request(Frame::sen(cmd::ANC_SET, vec![mode, state])))
    }

    pub fn codec(&mut self) -> Result<Codec, Error> {
        let frame = self.get(cmd::CODEC_GET)?;
        Ok(Codec::from_u8(byte_at(&frame, 0)?))
    }

    pub fn sample_rate(&mut self) -> Result<u32, Error> {
        let frame = self.get(cmd::SAMPLE_RATE_GET)?;
        let bytes = frame
            .payload
            .get(..4)
            .ok_or(Error::Decode(DecodeError::Length(frame.payload.len())))?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn audio_mode(&mut self) -> Result<u16, Error> {
        let frame = self.get(cmd::AUDIO_MODE_GET)?;
        word_at(&frame, 0)
    }

    pub fn eq_config(&mut self) -> Result<EqConfig, Error> {
        Ok(EqConfig::parse(&self.get(cmd::EQ_CONFIG_GET)?.payload)?)
    }

    pub fn eq_gain(&mut self, band: u8) -> Result<SliderGain, Error> {
        Ok(SliderGain::parse(
            &self.get_with(cmd::EQ_SLIDER_GET, vec![band])?.payload,
        )?)
    }

    pub fn set_eq_gain(&mut self, band: u8, gain_db: f32) -> Result<(), Error> {
        let value = (gain_db * 10.0).round().clamp(-128.0, 127.0) as i8;
        ignore_no_change(self.request(Frame::sen(cmd::EQ_SLIDER_SET, vec![band, value as u8])))
    }

    pub fn set_eq_gain_and_enable(&mut self, band: u8, gain_db: f32) -> Result<(), Error> {
        self.set_eq_gain(band, gain_db)?;
        self.set_audio_mode(1)
    }

    pub fn set_audio_mode(&mut self, mode: u16) -> Result<(), Error> {
        ignore_no_change(self.request(Frame::sen(cmd::AUDIO_MODE_SET, mode.to_be_bytes().to_vec())))
    }

    /// Gains first, then audio mode 1: the headset ignores the user EQ in any
    /// other mode.
    pub fn apply_eq(&mut self, gains_db: [f32; 5]) -> Result<(), Error> {
        for (band, gain) in gains_db.iter().enumerate() {
            self.set_eq_gain(band as u8, *gain)?;
        }
        self.set_audio_mode(1)
    }

    /// Neutral is audio mode 0, not a flat curve.
    pub fn clear_eq(&mut self) -> Result<(), Error> {
        self.set_audio_mode(0)
    }

    pub fn bass_boost(&mut self) -> Result<bool, Error> {
        Ok(byte_at(&self.get(cmd::EQ_SUBMODE_GET)?, 0)? != 0)
    }

    pub fn set_bass_boost(&mut self, on: bool) -> Result<(), Error> {
        ignore_no_change(self.request(Frame::sen(cmd::EQ_SUBMODE_SET, vec![u8::from(on)])))
    }

    pub fn on_head_detection(&mut self) -> Result<bool, Error> {
        Ok(byte_at(&self.get(cmd::ON_HEAD_GET)?, 0)? != 0)
    }

    pub fn physical_state(&mut self) -> Result<u8, Error> {
        byte_at(&self.get(cmd::PHYSICAL_STATE_GET)?, 0)
    }

    pub fn touch_locked(&mut self) -> Result<bool, Error> {
        Ok(byte_at(&self.get(cmd::TOUCH_LOCK_GET)?, 0)? != 0)
    }

    pub fn set_touch_lock(&mut self, locked: bool) -> Result<(), Error> {
        ignore_no_change(self.request(Frame::sen(cmd::TOUCH_LOCK_SET, vec![u8::from(locked)])))
    }

    pub fn auto_power_off(&mut self) -> Result<u16, Error> {
        let frame = self.get_with(cmd::TIMER_GET, vec![0])?;
        Ok(u16::from_be_bytes([
            byte_at(&frame, 1)?,
            byte_at(&frame, 2)?,
        ]))
    }

    pub fn bluetooth_mode(&mut self) -> Result<u8, Error> {
        byte_at(&self.get(cmd::BT_MODE_GET)?, 0)
    }

    pub fn paired_size(&mut self) -> Result<u16, Error> {
        let frame = self.get(cmd::PAIRED_SIZE_GET)?;
        word_at(&frame, 0)
    }

    pub fn paired_device(&mut self, index: u8) -> Result<PairedDevice, Error> {
        Ok(PairedDevice::parse(
            &self.get_with(cmd::PAIRED_DEVICE_GET, vec![index])?.payload,
        )?)
    }

    pub fn paired_devices(&mut self) -> Result<Vec<PairedDevice>, Error> {
        let size = self.paired_size()?;
        let mut devices = Vec::with_capacity(size as usize);
        for index in 0..size as u8 {
            devices.push(self.paired_device(index)?);
        }
        Ok(devices)
    }

    pub fn own_index(&mut self) -> Result<u8, Error> {
        byte_at(&self.get(cmd::PAIRED_OWN_INDEX_GET)?, 0)
    }

    pub fn connect_paired(&mut self, index: u8) -> Result<(), Error> {
        ignore_no_change(self.request(Frame::sen(cmd::PAIRED_CONNECT, vec![index])))
    }

    pub fn disconnect_paired(&mut self, index: u8) -> Result<(), Error> {
        ignore_no_change(self.request(Frame::sen(cmd::PAIRED_DISCONNECT, vec![index])))
    }
}

/// The headset ignores `ANC_Transparency` while adaptive noise control is on
/// (or while ANC is off), so a slider move has to enter custom mode first.
pub fn transparency_needs_custom(info: &Info) -> bool {
    info.anc_status == Some(0)
        || info
            .anc
            .as_deref()
            .is_some_and(|anc| anc.contains("adaptive=1"))
}

/// Applies a pushed notification to the cached snapshot. Returns false when the
/// frame is not understood, so the caller can fall back to a full refresh.
pub fn apply_notification(info: &mut Info, frame: &Frame) -> bool {
    let payload = frame.payload.as_slice();
    let first = payload.first().copied();
    match frame.command {
        0x0683 => info.battery_percent = first,
        0x0682 => {
            info.charging = Charging::parse(payload)
                .ok()
                .map(|value| value.label().to_owned())
        }
        0x1a81 => info.anc = AncModes::parse(payload).ok().map(|value| value.to_string()),
        0x1a83 => info.transparency = first,
        0x1a85 => info.anc_status = first,
        0x0884 => {
            if payload.len() >= 2 {
                let mode = u16::from_be_bytes([payload[0], payload[1]]);
                info.audio_mode = Some(gaia::audio_mode_name(mode).to_owned());
            }
        }
        0x089a => {
            if payload.len() >= 4 {
                info.sample_rate_hz = Some(u32::from_be_bytes([
                    payload[0], payload[1], payload[2], payload[3],
                ]));
            }
        }
        0x0880 => {
            if let Some(codec) = first {
                info.codec = Some(Codec::from_u8(codec).to_string());
            }
        }
        0x0502 => {
            if let Some(state) = first {
                info.physical_state = Some(gaia::PhysicalState::from_u8(state).to_string());
            }
        }
        0x1707 => info.touch_locked = first.map(|value| value != 0),
        0x1082 => {
            info.eq_gains_db = payload
                .iter()
                .map(|value| f32::from(*value as i8) / 10.0)
                .collect();
        }
        _ => return false,
    }
    true
}

pub const PROPERTIES: [&str; 21] = [
    "firmware",
    "model",
    "serial",
    "hardware",
    "battery",
    "charging",
    "anc",
    "anc-status",
    "transparency",
    "codec",
    "sample-rate",
    "audio-mode",
    "on-head",
    "physical",
    "touch-lock",
    "auto-off",
    "bt-mode",
    "bass-boost",
    "own-index",
    "eq",
    "paired",
];

pub fn known_property(name: &str) -> bool {
    PROPERTIES.contains(&name)
}

impl Session {
    /// Named read used by both CLIs; keeps the property vocabulary in one place.
    pub fn property(&mut self, name: &str) -> Result<String, Error> {
        let value = match name {
            "firmware" => self.firmware()?.to_string(),
            "model" => self.model()?,
            "serial" => self.serial()?,
            "hardware" => self.hardware_revision()?.to_string(),
            "battery" => self.battery()?.to_string(),
            "charging" => self.charging()?.label().to_owned(),
            "anc" => self.anc_modes()?.to_string(),
            "anc-status" => self.anc_status()?.to_string(),
            "transparency" => self.transparency()?.to_string(),
            "codec" => self.codec()?.to_string(),
            "sample-rate" => self.sample_rate()?.to_string(),
            "audio-mode" => gaia::audio_mode_name(self.audio_mode()?).to_owned(),
            "on-head" => self.on_head_detection()?.to_string(),
            "physical" => gaia::PhysicalState::from_u8(self.physical_state()?).to_string(),
            "touch-lock" => self.touch_locked()?.to_string(),
            "auto-off" => self.auto_power_off()?.to_string(),
            "bt-mode" => self.bluetooth_mode()?.to_string(),
            "bass-boost" => self.bass_boost()?.to_string(),
            "own-index" => self.own_index()?.to_string(),
            "eq" => {
                let config = self.eq_config()?;
                let mut gains = Vec::new();
                for band in 0..config.bands {
                    gains.push(format!("{:+.1}", self.eq_gain(band)?.db()));
                }
                format!(
                    "{} bands, {:.1}..{:+.1} dB: {}",
                    config.bands,
                    config.min_db,
                    config.max_db,
                    gains.join(" ")
                )
            }
            "paired" => {
                let mut lines = Vec::new();
                for device in self.paired_devices()? {
                    lines.push(format!(
                        "[{}] {} {}",
                        device.index,
                        if device.connected {
                            "connected"
                        } else {
                            "disconnected"
                        },
                        device.name
                    ));
                }
                lines.join("\n")
            }
            other => return Err(Error::UnknownProperty(other.to_owned())),
        };
        Ok(value)
    }
}

fn ignore_no_change(result: Result<Frame, Error>) -> Result<(), Error> {
    match result {
        Ok(_) => Ok(()),
        Err(Error::Device {
            code: NO_CHANGE, ..
        }) => Ok(()),
        Err(error) => Err(error),
    }
}

fn byte_at(frame: &Frame, index: usize) -> Result<u8, Error> {
    frame.payload.get(index).copied().ok_or_else(|| {
        Error::Decode(DecodeError::Short {
            need: index + 1,
            got: frame.payload.len(),
        })
    })
}

fn word_at(frame: &Frame, index: usize) -> Result<u16, Error> {
    let bytes = frame.payload.get(index..index + 2).ok_or_else(|| {
        Error::Decode(DecodeError::Short {
            need: index + 2,
            got: frame.payload.len(),
        })
    })?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn string_payload(payload: &[u8]) -> String {
    let end = payload
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(payload.len());
    String::from_utf8_lossy(&payload[..end]).into_owned()
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Info {
    pub model: Option<String>,
    pub firmware: Option<String>,
    pub hardware_revision: Option<u16>,
    pub serial: Option<String>,
    pub gaia_api: Option<String>,
    pub battery_percent: Option<u8>,
    pub charging: Option<String>,
    pub anc: Option<String>,
    pub anc_status: Option<u8>,
    pub transparency: Option<u8>,
    pub codec: Option<String>,
    pub sample_rate_hz: Option<u32>,
    pub audio_mode: Option<String>,
    pub on_head_detection: Option<bool>,
    pub physical_state: Option<String>,
    pub touch_locked: Option<bool>,
    pub auto_power_off_s: Option<u16>,
    pub bluetooth_mode: Option<String>,
    pub bass_boost: Option<bool>,
    pub eq_bands: Option<u8>,
    pub eq_range_db: Option<(f32, f32)>,
    pub eq_gains_db: Vec<f32>,
    pub paired: Vec<PairedEntry>,
    pub own_index: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PairedEntry {
    pub index: u8,
    pub name: String,
    pub connected: bool,
    pub priority: u8,
}

impl Info {
    /// Every field is optional: a firmware that rejects one command still yields
    /// the rest of the snapshot.
    #[allow(clippy::field_reassign_with_default)] // every field is read independently
    pub fn read(session: &mut Session) -> Self {
        let mut info = Info::default();
        info.model = session.model().ok();
        info.firmware = session.firmware().ok().map(|value| value.to_string());
        info.hardware_revision = session.hardware_revision().ok();
        info.serial = session.serial().ok();
        info.gaia_api = session
            .gaia_api()
            .ok()
            .map(|(major, minor)| format!("{major}.{minor}"));
        info.battery_percent = session.battery().ok();
        info.charging = session
            .charging()
            .ok()
            .map(|value| value.label().to_owned());
        info.anc = session.anc_modes().ok().map(|modes| modes.to_string());
        info.anc_status = session.anc_status().ok();
        info.transparency = session.transparency().ok();
        info.codec = session.codec().ok().map(|value| value.to_string());
        info.sample_rate_hz = session.sample_rate().ok();
        info.audio_mode = session
            .audio_mode()
            .ok()
            .map(|mode| gaia::audio_mode_name(mode).to_owned());
        info.on_head_detection = session.on_head_detection().ok();
        info.physical_state = session
            .physical_state()
            .ok()
            .map(|value| gaia::PhysicalState::from_u8(value).to_string());
        info.touch_locked = session.touch_locked().ok();
        info.auto_power_off_s = session.auto_power_off().ok();
        info.bluetooth_mode = session.bluetooth_mode().ok().map(|mode| {
            if mode == 0 {
                "better audio (Hi-Res)".to_owned()
            } else {
                "better compatibility".to_owned()
            }
        });
        info.bass_boost = session.bass_boost().ok();
        if let Ok(config) = session.eq_config() {
            info.eq_bands = Some(config.bands);
            info.eq_range_db = Some((config.min_db, config.max_db));
            for band in 0..config.bands {
                if let Ok(gain) = session.eq_gain(band) {
                    info.eq_gains_db.push(gain.db());
                }
            }
        }
        info.own_index = session.own_index().ok();
        if let Ok(devices) = session.paired_devices() {
            info.paired = devices
                .into_iter()
                .map(|device| PairedEntry {
                    index: device.index,
                    name: device.name,
                    connected: device.connected,
                    priority: device.priority,
                })
                .collect();
        }
        info
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let field = |out: &mut String, label: &str, value: String| {
            let _ = writeln!(out, "{label:<18} {value}");
        };
        field(
            &mut out,
            "model",
            self.model.clone().unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "firmware",
            self.firmware.clone().unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "hardware",
            self.hardware_revision
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "serial",
            self.serial.clone().unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "gaia api",
            self.gaia_api.clone().unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "battery",
            match (self.battery_percent, &self.charging) {
                (Some(percent), Some(state)) => format!("{percent}% ({state})"),
                (Some(percent), None) => format!("{percent}%"),
                _ => "?".into(),
            },
        );
        field(
            &mut out,
            "anc",
            self.anc.clone().unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "anc status",
            self.anc_status
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "transparency",
            self.transparency
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "codec",
            self.codec.clone().unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "sample rate",
            self.sample_rate_hz
                .map(|hz| format!("{hz} Hz"))
                .unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "audio mode",
            self.audio_mode.clone().unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "on head",
            self.on_head_detection
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "physical",
            self.physical_state.clone().unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "touch locked",
            self.touch_locked
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "auto power off",
            self.auto_power_off_s
                .map(|v| format!("{v} s"))
                .unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "bluetooth mode",
            self.bluetooth_mode.clone().unwrap_or_else(|| "?".into()),
        );
        field(
            &mut out,
            "bass boost",
            self.bass_boost
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
        );
        if let (Some(bands), Some((min, max))) = (self.eq_bands, self.eq_range_db) {
            let gains: Vec<String> = self
                .eq_gains_db
                .iter()
                .map(|gain| format!("{gain:+.1}"))
                .collect();
            field(
                &mut out,
                "eq",
                format!("{bands} bands, {min:.1}..{max:+.1} dB: {}", gains.join(" ")),
            );
        }
        for device in &self.paired {
            let marker = if Some(device.index) == self.own_index {
                " *"
            } else {
                ""
            };
            let state = if device.connected {
                "connected"
            } else {
                "disconnected"
            };
            field(
                &mut out,
                format!("paired[{:02}]", device.index).as_str(),
                format!("{state:<12} {}{marker}", device.name),
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(command: u16, payload: &[u8]) -> Frame {
        Frame::new(gaia::SENNHEISER, command, payload.to_vec())
    }

    #[test]
    fn notifications_update_the_cached_snapshot() {
        let mut info = Info::default();

        assert!(apply_notification(&mut info, &frame(0x0683, &[0x64])));
        assert_eq!(info.battery_percent, Some(100));

        assert!(apply_notification(&mut info, &frame(0x0682, &[1, 0])));
        assert_eq!(info.charging.as_deref(), Some("charging"));

        assert!(apply_notification(
            &mut info,
            &frame(0x1a81, &[1, 0, 2, 0, 3, 1])
        ));
        assert_eq!(
            info.anc.as_deref(),
            Some("adaptive=1 anti-wind=0 comfort=0")
        );

        assert!(apply_notification(&mut info, &frame(0x1a83, &[69])));
        assert_eq!(info.transparency, Some(69));

        assert!(apply_notification(&mut info, &frame(0x1a85, &[1])));
        assert_eq!(info.anc_status, Some(1));

        assert!(apply_notification(&mut info, &frame(0x0884, &[0, 1])));
        assert_eq!(info.audio_mode.as_deref(), Some("user EQ"));

        assert!(apply_notification(
            &mut info,
            &frame(0x089a, &[0, 0, 0xbb, 0x80])
        ));
        assert_eq!(info.sample_rate_hz, Some(48_000));

        assert!(apply_notification(&mut info, &frame(0x0880, &[5])));
        assert_eq!(info.codec.as_deref(), Some("aptX HD"));

        assert!(apply_notification(&mut info, &frame(0x0502, &[3])));
        assert_eq!(info.physical_state.as_deref(), Some("on head"));

        assert!(apply_notification(&mut info, &frame(0x1707, &[1])));
        assert_eq!(info.touch_locked, Some(true));

        assert!(apply_notification(
            &mut info,
            &frame(0x1082, &[0, 0x14, 0x19, 0x0f, 0xec])
        ));
        assert_eq!(info.eq_gains_db, vec![0.0, 2.0, 2.5, 1.5, -2.0]);
    }

    #[test]
    fn unknown_notifications_are_reported_as_unhandled() {
        let mut info = Info::default();
        assert!(!apply_notification(&mut info, &frame(0x1885, &[0])));
        assert!(!apply_notification(&mut info, &frame(0x0898, &[0])));
        assert_eq!(info, Info::default());
    }
}

#[cfg(test)]
mod preset_detection_tests {
    use super::*;

    #[test]
    fn detects_the_matching_curve() {
        assert_eq!(preset_for_gains(&[0.0, 2.0, 2.5, 1.5, -2.0]), Some("rock"));
        assert_eq!(preset_for_gains(&[-3.2, 0.0, 2.2, 2.2, 0.0]), Some("jazz"));
        assert_eq!(preset_for_gains(&[1.0, 1.0, 1.0, 1.0, 1.0]), None);
        assert_eq!(preset_for_gains(&[0.0, 2.0]), None, "needs five bands");
    }

    #[test]
    fn now_playing_label_falls_back_to_the_player() {
        let full = NowPlaying {
            player: "chromium".to_owned(),
            artist: Some("Metallica".to_owned()),
            title: Some("One".to_owned()),
            playing: true,
        };
        assert_eq!(full.label(), "Metallica — One");
        let bare = NowPlaying {
            player: "mpv".to_owned(),
            ..NowPlaying::default()
        };
        assert_eq!(bare.label(), "mpv");
    }

    #[test]
    fn tolerates_stored_rounding() {
        assert_eq!(
            preset_for_gains(&[0.04, 2.05, 2.45, 1.5, -1.96]),
            Some("rock")
        );
    }
}

#[cfg(test)]
mod transparency_tests {
    use super::*;

    #[test]
    fn adaptive_and_off_need_custom_mode() {
        let mut info = Info {
            anc: Some("adaptive=1 anti-wind=0 comfort=0".to_owned()),
            anc_status: Some(1),
            ..Info::default()
        };
        assert!(transparency_needs_custom(&info));

        info.anc_status = Some(0);
        info.anc = Some("adaptive=0 anti-wind=0 comfort=0".to_owned());
        assert!(transparency_needs_custom(&info));

        info.anc_status = Some(1);
        assert!(!transparency_needs_custom(&info));
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;

    #[test]
    fn table_matches_the_sennheiser_app() {
        assert_eq!(PRESETS.len(), 7);
        assert_eq!(preset("rock").unwrap().gains_db, [0.0, 2.0, 2.5, 1.5, -2.0]);
        assert_eq!(preset("Rock").unwrap().name, "rock");
        assert_eq!(preset("JAZZ").unwrap().gains_db, [-3.2, 0.0, 2.2, 2.2, 0.0]);
        assert!(preset("dubstep").is_none());
    }

    #[test]
    fn every_preset_is_inside_the_measured_range() {
        for preset in &PRESETS {
            for gain in preset.gains_db {
                assert!((-6.0..=6.0).contains(&gain), "{} band {gain}", preset.name);
            }
        }
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use std::io;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct State {
        // Each inner vector is released by one request; several entries model duplicates.
        queued: std::collections::VecDeque<Vec<Vec<u8>>>,
        available: std::collections::VecDeque<Vec<u8>>,
        sent: Vec<Vec<u8>>,
        eof: bool,
    }

    #[derive(Clone, Default)]
    struct MockTransport {
        state: Arc<Mutex<State>>,
    }

    impl MockTransport {
        fn reply(&self, frame: Frame) {
            let mut state = self.state.lock().unwrap();
            let raw = frame.encode().unwrap();
            if let Some(replies) = state.queued.back_mut() {
                replies.push(raw);
            } else {
                state.queued.push_back(vec![raw]);
            }
        }

        fn reply_next(&self, frame: Frame) {
            self.state
                .lock()
                .unwrap()
                .queued
                .push_back(vec![frame.encode().unwrap()]);
        }

        /// Unsolicited traffic: readable without any request being sent.
        fn push_now(&self, frame: Frame) {
            self.state
                .lock()
                .unwrap()
                .available
                .push_back(frame.encode().unwrap());
        }

        fn eof(&self) {
            self.state.lock().unwrap().eof = true;
        }

        fn sent(&self) -> Vec<Vec<u8>> {
            self.state.lock().unwrap().sent.clone()
        }
    }

    impl Transport for MockTransport {
        fn send(&self, bytes: &[u8]) -> io::Result<()> {
            let mut state = self.state.lock().unwrap();
            state.sent.push(bytes.to_vec());
            if let Some(replies) = state.queued.pop_front() {
                state.available.extend(replies);
            }
            Ok(())
        }

        fn recv(&self, buffer: &mut [u8], _timeout: Duration) -> io::Result<Option<usize>> {
            let mut state = self.state.lock().unwrap();
            if state.eof {
                return Ok(Some(0));
            }
            let Some(next) = state.available.pop_front() else {
                return Ok(None);
            };
            buffer[..next.len()].copy_from_slice(&next);
            Ok(Some(next.len()))
        }
    }

    fn session_with(transport: &MockTransport) -> Session {
        let mut session = Session::from_transport(Box::new(transport.clone()), 1);
        session.timeout = Duration::from_millis(20);
        session
    }

    #[test]
    fn request_skips_notifications_and_returns_the_matching_response() {
        let transport = MockTransport::default();
        transport.reply(Frame::sen(0x0683, vec![0x64]));
        transport.reply(Frame::sen(0x0703, vec![0x64]));
        let mut session = session_with(&transport);

        let reply = session
            .request(Frame::sen(0x0603, Vec::new()))
            .expect("response");
        assert_eq!(reply.payload, vec![0x64]);
        assert_eq!(
            transport.sent()[0],
            vec![0xff, 0x03, 0x00, 0x00, 0x04, 0x95, 0x06, 0x03]
        );

        let notifications = session.take_notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].command, 0x0683);
    }

    #[test]
    fn error_frames_become_device_errors() {
        let transport = MockTransport::default();
        transport.reply(Frame::sen(0x0783, vec![0x05]));
        let mut session = session_with(&transport);

        match session.request(Frame::sen(0x0603, Vec::new())) {
            Err(Error::Device { command, code }) => {
                assert_eq!(command, 0x0603);
                assert_eq!(code, 0x05);
            }
            other => panic!("expected a device error, got {other:?}"),
        }
    }

    #[test]
    fn silence_is_a_timeout_and_eof_is_a_closed_link() {
        let silent = MockTransport::default();
        let mut session = session_with(&silent);
        assert!(matches!(
            session.request(Frame::sen(0x0603, Vec::new())),
            Err(Error::Timeout(0x0603))
        ));

        let closed = MockTransport::default();
        closed.eof();
        let mut session = session_with(&closed);
        assert!(matches!(
            session.request(Frame::sen(0x0603, Vec::new())),
            Err(Error::Closed(0x0603))
        ));
    }

    #[test]
    fn truncated_payloads_are_errors_not_panics() {
        macro_rules! check {
            ($($call:expr),* $(,)?) => {$(
                let transport = MockTransport::default();
                transport.reply(Frame::sen(0x0001, Vec::new()));
                let mut session = session_with(&transport);
                assert!(
                    $call(&mut session).is_err(),
                    "{} on an empty payload must error",
                    stringify!($call)
                );
            )*};
        }

        check!(
            |s: &mut Session| s.hardware_revision(),
            |s: &mut Session| s.battery(),
            |s: &mut Session| s.charging(),
            |s: &mut Session| s.anc_modes(),
            |s: &mut Session| s.anc_status(),
            |s: &mut Session| s.transparency(),
            |s: &mut Session| s.codec(),
            |s: &mut Session| s.audio_mode(),
            |s: &mut Session| s.sample_rate(),
            |s: &mut Session| s.bass_boost(),
            |s: &mut Session| s.on_head_detection(),
            |s: &mut Session| s.physical_state(),
            |s: &mut Session| s.touch_locked(),
            |s: &mut Session| s.auto_power_off(),
            |s: &mut Session| s.bluetooth_mode(),
            |s: &mut Session| s.paired_size(),
            |s: &mut Session| s.own_index(),
            |s: &mut Session| s.eq_gain(0),
        );
    }

    #[test]
    fn duplicate_replies_do_not_leak_into_the_next_request() {
        let transport = MockTransport::default();
        transport.reply(Frame::sen(0x0703, vec![0x64]));
        transport.reply(Frame::sen(0x0703, vec![0x64]));
        let mut session = session_with(&transport);

        let first = session
            .request(Frame::sen(0x0603, Vec::new()))
            .expect("first");
        assert_eq!(first.payload, vec![0x64]);

        assert!(
            matches!(
                session.request(Frame::sen(0x0603, Vec::new())),
                Err(Error::Timeout(0x0603))
            ),
            "the duplicated reply must be dropped, not reused"
        );
    }

    #[test]
    fn single_eq_write_changes_the_band_then_enables_user_eq() {
        let transport = MockTransport::default();
        transport.reply_next(Frame::sen(0x1101, Vec::new()));
        transport.reply_next(Frame::sen(0x0903, Vec::new()));
        let mut session = session_with(&transport);

        session.set_eq_gain_and_enable(2, -0.5).expect("EQ write");

        assert_eq!(
            transport.sent(),
            vec![
                Frame::sen(cmd::EQ_SLIDER_SET, vec![2, 0xfb])
                    .encode()
                    .unwrap(),
                Frame::sen(cmd::AUDIO_MODE_SET, vec![0, 1])
                    .encode()
                    .unwrap(),
            ]
        );
    }

    #[test]
    fn preset_writes_all_bands_before_enabling_user_eq() {
        let transport = MockTransport::default();
        for _ in 0..5 {
            transport.reply_next(Frame::sen(0x1101, Vec::new()));
        }
        transport.reply_next(Frame::sen(0x0903, Vec::new()));
        let mut session = session_with(&transport);

        session.apply_eq(PRESETS[0].gains_db).expect("preset");

        let sent = transport.sent();
        assert_eq!(sent.len(), 6);
        for (band, raw) in sent[..5].iter().enumerate() {
            let gain = (PRESETS[0].gains_db[band] * 10.0).round() as i8;
            assert_eq!(
                *raw,
                Frame::sen(cmd::EQ_SLIDER_SET, vec![band as u8, gain as u8])
                    .encode()
                    .unwrap()
            );
        }
        assert_eq!(
            sent[5],
            Frame::sen(cmd::AUDIO_MODE_SET, vec![0, 1])
                .encode()
                .unwrap()
        );
    }

    #[test]
    fn failed_eq_band_does_not_enable_user_eq() {
        let transport = MockTransport::default();
        transport.reply(Frame::sen(0x1181, vec![1]));
        let mut session = session_with(&transport);

        assert!(session.set_eq_gain_and_enable(0, 1.0).is_err());
        assert_eq!(transport.sent().len(), 1);
    }

    #[test]
    fn poll_returns_pushed_frames_without_sending() {
        let transport = MockTransport::default();
        transport.push_now(Frame::sen(0x1a83, vec![0x45]));
        let mut session = session_with(&transport);

        let notifications = session.poll(Duration::from_millis(20)).expect("poll");
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].payload, vec![0x45]);
        assert!(transport.sent().is_empty());
    }
}
