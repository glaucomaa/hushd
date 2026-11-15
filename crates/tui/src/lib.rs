//! TUI backend boundary. Everything the UI does goes through [`Backend`], so the
//! screens can be developed and tested against [`MockBackend`] without a headset.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub mod app;
pub mod render;

use ipc::{AncMode, Event, Request, Response, Status};

pub trait Backend {
    fn status(&mut self) -> Result<Status, String>;
    fn dispatch(&mut self, request: Request) -> Result<Response, String>;
}

pub struct DaemonBackend {
    socket: PathBuf,
    events: Arc<Mutex<VecDeque<Event>>>,
}

impl DaemonBackend {
    pub fn new(socket: PathBuf) -> Self {
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let watcher_events = Arc::clone(&events);
        let watcher_socket = socket.clone();
        std::thread::spawn(move || loop {
            let sink = Arc::clone(&watcher_events);
            let _ = ipc::watch(&watcher_socket, move |event| {
                if let Ok(mut events) = sink.lock() {
                    if events.len() == 100 {
                        events.pop_front();
                    }
                    events.push_back(event);
                }
            });
            std::thread::sleep(std::time::Duration::from_secs(1));
        });
        Self { socket, events }
    }

    pub fn drain_events(&mut self) -> Vec<Event> {
        match self.events.lock() {
            Ok(mut events) => events.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }
}

impl Backend for DaemonBackend {
    fn status(&mut self) -> Result<Status, String> {
        match ipc::request(&self.socket, &Request::Status).map_err(|error| error.to_string())? {
            Response::Status(status) => Ok(status),
            Response::Error { message } => Err(message),
            other => Err(format!("unexpected reply: {other:?}")),
        }
    }

    fn dispatch(&mut self, request: Request) -> Result<Response, String> {
        ipc::request(&self.socket, &request).map_err(|error| error.to_string())
    }
}

pub struct MockBackend {
    status: Status,
    pub requests: Vec<Request>,
}

impl Default for MockBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MockBackend {
    pub fn new() -> Self {
        Self {
            status: sample_status(),
            requests: Vec::new(),
        }
    }

    pub fn set_status(&mut self, status: Status) {
        self.status = status;
    }
}

impl Backend for MockBackend {
    fn status(&mut self) -> Result<Status, String> {
        Ok(self.status.clone())
    }

    fn dispatch(&mut self, request: Request) -> Result<Response, String> {
        if let Err(message) = request.validate() {
            return Ok(Response::Error { message });
        }
        self.requests.push(request.clone());
        match request {
            Request::Status => Ok(Response::Status(self.status.clone())),
            Request::Anc { mode } => {
                self.status.info.anc = Some(match mode {
                    AncMode::Adaptive => "adaptive=1 anti-wind=0 comfort=0".to_owned(),
                    AncMode::Custom => "adaptive=0 anti-wind=0 comfort=0".to_owned(),
                    AncMode::Off => "adaptive=0 anti-wind=0 comfort=0".to_owned(),
                });
                self.status.info.anc_status = Some(u8::from(matches!(
                    mode,
                    AncMode::Adaptive | AncMode::Custom
                )));
                Ok(Response::Ok)
            }
            Request::Transparency { level } => {
                self.status.info.transparency = Some(level.min(100));
                Ok(Response::Ok)
            }
            Request::EqGain { band, centibel } => {
                let db = centibel as f32 / 10.0;
                if let Some(slot) = self.status.info.eq_gains_db.get_mut(band as usize) {
                    *slot = db;
                }
                self.status.info.audio_mode = Some("user EQ".to_owned());
                Ok(Response::Ok)
            }
            Request::BassBoost { enabled } => {
                self.status.info.bass_boost = Some(enabled);
                Ok(Response::Ok)
            }
            Request::TouchLock { enabled } => {
                self.status.info.touch_locked = Some(enabled);
                Ok(Response::Ok)
            }
            _ => Ok(Response::Ok),
        }
    }
}

fn sample_status() -> Status {
    Status {
        connected: true,
        channel: Some(1),
        error: None,
        info: ipc::Info {
            model: Some("M4AEBT Graphite".to_owned()),
            firmware: Some("3.38.3".to_owned()),
            battery_percent: Some(87),
            charging: Some("discharging".to_owned()),
            anc: Some("adaptive=0 anti-wind=0 comfort=0".to_owned()),
            anc_status: Some(1),
            transparency: Some(50),
            codec: Some("aptX HD".to_owned()),
            sample_rate_hz: Some(48_000),
            eq_bands: Some(5),
            eq_range_db: Some((-6.0, 6.0)),
            eq_gains_db: vec![0.0, 0.0, 0.0, 0.0, 0.0],
            ..Default::default()
        },
        audio: None,
        now_playing: None,
        address: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_applies_writes_and_records_them() {
        let mut backend = MockBackend::new();
        assert!(matches!(
            backend.dispatch(Request::Anc {
                mode: AncMode::Adaptive
            }),
            Ok(Response::Ok)
        ));
        assert_eq!(backend.requests.len(), 1);

        let status = backend.status().unwrap();
        assert_eq!(status.info.anc_status, Some(1));
        assert!(status.info.anc.unwrap().starts_with("adaptive=1"));
    }

    #[test]
    fn mock_rejects_invalid_writes_and_enables_user_eq() {
        let mut backend = MockBackend::new();
        assert!(matches!(
            backend.dispatch(Request::Transparency { level: 140 }),
            Ok(Response::Error { .. })
        ));
        assert!(matches!(
            backend.dispatch(Request::EqGain {
                band: 2,
                centibel: 250,
            }),
            Ok(Response::Error { .. })
        ));
        assert!(backend.requests.is_empty());

        backend
            .dispatch(Request::EqGain {
                band: 2,
                centibel: 25,
            })
            .unwrap();
        let status = backend.status().unwrap();
        assert_eq!(status.info.eq_gains_db[2], 2.5);
        assert_eq!(status.info.audio_mode.as_deref(), Some("user EQ"));
    }
}
