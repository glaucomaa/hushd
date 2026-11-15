//! Newline-delimited JSON between `m4d` and its clients. One request per
//! connection, except `Watch`, which turns the connection into an event stream.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

const REQUEST_IO_TIMEOUT: Duration = Duration::from_secs(5);

use serde::{Deserialize, Serialize};

pub use m4::Info;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AncMode {
    Adaptive,
    Custom,
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum Request {
    Status,
    Watch,
    Get { property: String },
    Anc { mode: AncMode },
    Transparency { level: u8 },
    EqGain { band: u8, centibel: i16 },
    Preset { name: String },
    BassBoost { enabled: bool },
    TouchLock { enabled: bool },
    Audio { set_profile: Option<String> },
    ConnectPeer { index: u8 },
    DisconnectPeer { index: u8 },
    Shutdown,
}

impl Request {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Get { property } if !m4::known_property(property) => {
                Err(format!("unknown property {property}"))
            }
            Self::Transparency { level } if *level > 100 => {
                Err(format!("transparency level must be 0..100, got {level}"))
            }
            Self::EqGain { band, .. } if *band > 4 => {
                Err(format!("EQ band must be 0..4, got {band}"))
            }
            Self::EqGain { centibel, .. } if !(-60..=60).contains(centibel) => {
                Err(format!("EQ gain must be within ±6 dB, got {centibel}"))
            }
            Self::Preset { name }
                if !matches!(name.as_str(), "neutral" | "off") && m4::preset(name).is_none() =>
            {
                Err(format!("unknown preset {name:?}; run `m4 preset list`"))
            }
            Self::Audio {
                set_profile: Some(profile),
            } if profile.trim().is_empty() => Err("audio profile must not be empty".to_owned()),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // one message at a time, boxing buys nothing
pub enum Response {
    Status(Status),
    Audio(AudioState),
    Value { value: String },
    Ok,
    Error { message: String },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioState {
    pub card: String,
    pub address: String,
    pub profile: String,
    pub codec: Option<String>,
    pub sample_rate_hz: Option<u32>,
    pub channels: Option<u8>,
    pub format: Option<String>,
    pub state: Option<String>,
    pub is_headset: bool,
    pub autoswitch: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub connected: bool,
    pub channel: Option<u8>,
    pub error: Option<String>,
    pub info: Info,
    #[serde(default)]
    pub audio: Option<AudioState>,
    #[serde(default)]
    pub now_playing: Option<m4::NowPlaying>,
    #[serde(default)]
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum Event {
    Connected { channel: u8 },
    Disconnected { reason: String },
    State { status: Status },
    Notification { command: u16, payload: Vec<u8> },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("daemon socket {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("malformed reply: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn socket_path() -> PathBuf {
    if let Some(explicit) = std::env::var_os("M4D_SOCKET") {
        return PathBuf::from(explicit);
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("m4d.sock");
    }
    PathBuf::from(format!("/tmp/m4d-{}.sock", std::process::id()))
}

fn connect(path: &Path) -> Result<UnixStream, Error> {
    UnixStream::connect(path).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}

pub fn request(path: &Path, request: &Request) -> Result<Response, Error> {
    request_with_timeout(path, request, REQUEST_IO_TIMEOUT)
}

fn request_with_timeout(
    path: &Path,
    request: &Request,
    timeout: Duration,
) -> Result<Response, Error> {
    let mut stream = connect(path)?;
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
    let line = serde_json::to_string(request)?;
    stream
        .write_all(line.as_bytes())
        .and_then(|()| stream.write_all(b"\n"))
        .and_then(|()| stream.flush())
        .map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
    let mut reply = String::new();
    BufReader::new(stream)
        .read_line(&mut reply)
        .map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
    if reply.trim().is_empty() {
        return Err(Error::Json(serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "daemon closed the socket without replying",
        ))));
    }
    Ok(serde_json::from_str(reply.trim_end())?)
}

pub fn watch(path: &Path, mut on_event: impl FnMut(Event)) -> Result<(), Error> {
    let mut stream = connect(path)?;
    stream
        .write_all(br#"{"request":"watch"}"#)
        .and_then(|()| stream.write_all(b"\n"))
        .and_then(|()| stream.flush())
        .map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
    for line in BufReader::new(stream).lines() {
        let line = line.map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
        if line.trim().is_empty() {
            continue;
        }
        on_event(serde_json::from_str(&line)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_through_json() {
        let requests = [
            Request::Status,
            Request::Watch,
            Request::Anc {
                mode: AncMode::Adaptive,
            },
            Request::Transparency { level: 55 },
            Request::EqGain {
                band: 3,
                centibel: -25,
            },
            Request::BassBoost { enabled: true },
            Request::Get {
                property: "battery".to_owned(),
            },
        ];
        for request in requests {
            request.validate().unwrap();
            let line = serde_json::to_string(&request).unwrap();
            assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), request);
        }
        assert_eq!(
            serde_json::to_string(&Request::Anc { mode: AncMode::Off }).unwrap(),
            r#"{"request":"anc","mode":"off"}"#
        );
    }

    #[test]
    fn validation_rejects_bad_values_before_any_io() {
        let invalid = [
            Request::Transparency { level: 101 },
            Request::EqGain {
                band: 5,
                centibel: 0,
            },
            Request::EqGain {
                band: 0,
                centibel: 61,
            },
            Request::Preset {
                name: "unknown".to_owned(),
            },
            Request::Get {
                property: "unknown".to_owned(),
            },
            Request::Audio {
                set_profile: Some("  ".to_owned()),
            },
        ];
        for request in invalid {
            assert!(request.validate().is_err(), "{request:?}");
        }

        for property in m4::PROPERTIES {
            assert!(Request::Get {
                property: property.to_owned(),
            }
            .validate()
            .is_ok());
        }
    }

    #[test]
    fn status_and_events_round_trip() {
        let status = Status {
            connected: true,
            channel: Some(1),
            error: None,
            info: Info::default(),
            audio: None,
            now_playing: None,
            address: None,
        };
        let line = serde_json::to_string(&Response::Status(status.clone())).unwrap();
        assert!(line.contains(r#""type":"status""#));
        assert!(line.contains(r#""connected":true"#));

        let event = Event::Disconnected {
            reason: "link closed".to_owned(),
        };
        let line = serde_json::to_string(&event).unwrap();
        assert_eq!(
            serde_json::from_str::<Event>(&line).unwrap(),
            Event::Disconnected {
                reason: "link closed".to_owned()
            }
        );
    }

    #[test]
    fn audio_state_round_trips() {
        let state = AudioState {
            card: "bluez_card.AA_BB_CC_DD_EE_FF".to_owned(),
            address: "AA:BB:CC:DD:EE:FF".to_owned(),
            profile: "a2dp-sink".to_owned(),
            codec: Some("aptx_hd".to_owned()),
            sample_rate_hz: Some(48_000),
            channels: Some(2),
            format: Some("s24le".to_owned()),
            state: Some("RUNNING".to_owned()),
            is_headset: false,
            autoswitch: Some(true),
        };
        let line = serde_json::to_string(&Response::Audio(state.clone())).unwrap();
        assert!(line.contains(r#""type":"audio""#));
        assert_eq!(
            serde_json::from_str::<Response>(&line).unwrap(),
            Response::Audio(state)
        );

        let request = Request::Audio {
            set_profile: Some("headset-head-unit".to_owned()),
        };
        let line = serde_json::to_string(&request).unwrap();
        assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), request);
    }

    #[test]
    fn socket_path_honours_override() {
        std::env::set_var("M4D_SOCKET", "/tmp/example.sock");
        assert_eq!(socket_path(), PathBuf::from("/tmp/example.sock"));
        std::env::remove_var("M4D_SOCKET");
    }

    #[test]
    fn silent_daemon_times_out_instead_of_blocking_forever() {
        let path = std::env::temp_dir().join(format!("m4-ipc-timeout-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(200));
        });

        let started = std::time::Instant::now();
        let result = request_with_timeout(&path, &Request::Status, Duration::from_millis(20));
        match result {
            Err(Error::Io { source, .. }) => assert!(matches!(
                source.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            )),
            other => panic!("expected I/O timeout, got {other:?}"),
        }
        assert!(started.elapsed() < Duration::from_millis(150));
        server.join().unwrap();
        let _ = std::fs::remove_file(path);
    }
}
