use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use ipc::{AncMode, Event, Request, Response, Status};
use m4::{Info, Session};

const SUBSCRIBED_FEATURES: [u8; 6] = [3, 4, 2, 11, 12, 13];
const REFRESH_EVERY: Duration = Duration::from_secs(60);
const DEFAULT_AUDIO_EVERY: Duration = Duration::from_secs(2);
const DEFAULT_TRACK_EVERY: Duration = Duration::from_secs(2);
const POLL_EVERY: Duration = Duration::from_millis(20);
const MAX_BACKOFF: Duration = Duration::from_secs(15);

pub enum Command {
    Request {
        request: Request,
        reply: Sender<Response>,
    },
}

pub fn spawn(
    rx: Receiver<Command>,
    state: Arc<RwLock<Status>>,
    subscribers: Arc<Mutex<Vec<Sender<Event>>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("m4d-worker".into())
        .spawn(move || {
            Worker {
                rx,
                state,
                subscribers,
            }
            .run()
        })
        .expect("worker thread must start")
}

struct Worker {
    rx: Receiver<Command>,
    state: Arc<RwLock<Status>>,
    subscribers: Arc<Mutex<Vec<Sender<Event>>>>,
}

impl Worker {
    fn run(&mut self) {
        self.run_with_connect(connect);
    }

    fn run_with_connect(&mut self, mut try_connect: impl FnMut() -> Result<Session, String>) {
        let audio_every = interval_from_env("M4_AUDIO_INTERVAL", DEFAULT_AUDIO_EVERY);
        let track_every = interval_from_env("M4_TRACK_INTERVAL", DEFAULT_TRACK_EVERY);
        let mut session: Option<Session> = None;
        let mut backoff = Duration::from_secs(1);
        let now = Instant::now();
        let mut last_refresh = now;
        let mut last_audio = now.checked_sub(audio_every).unwrap_or(now);
        let mut last_track = now.checked_sub(track_every).unwrap_or(now);
        let mut dirty = false;

        loop {
            let mut stop = false;
            while let Ok(Command::Request { request, reply }) = self.rx.try_recv() {
                let (should_stop, peer_change) = self.handle_command(&mut session, request, reply);
                stop = should_stop;
                if peer_change {
                    dirty = true;
                }
                if stop {
                    break;
                }
            }
            if stop {
                return;
            }

            match session.as_mut() {
                None => match try_connect() {
                    Ok(mut fresh) => {
                        let _ = fresh.subscribe(&SUBSCRIBED_FEATURES);
                        let info = Info::read(&mut fresh);
                        let channel = fresh.channel;
                        let address = fresh.address.clone();
                        self.publish(Status {
                            connected: true,
                            channel: Some(channel),
                            error: None,
                            info,
                            audio: read_audio(address.as_deref()),
                            now_playing: mpris::current(),
                            address: address.clone(),
                        });
                        eprintln!("m4d: connected on rfcomm channel {channel}");
                        self.broadcast(Event::Connected { channel });
                        session = Some(fresh);
                        backoff = Duration::from_secs(1);
                        last_refresh = Instant::now();
                        dirty = false;
                    }
                    Err(reason) => {
                        eprintln!("m4d: {reason}");
                        self.set_error(&reason);
                        match self.rx.recv_timeout(backoff) {
                            Ok(Command::Request { request, reply }) => {
                                let (stop, peer_change) =
                                    self.handle_command(&mut session, request, reply);
                                if peer_change {
                                    dirty = true;
                                }
                                if stop {
                                    return;
                                }
                            }
                            Err(RecvTimeoutError::Timeout) => {}
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                    }
                },
                Some(active) => match active.poll(POLL_EVERY) {
                    Ok(notifications) => {
                        let mut applied = false;
                        for frame in notifications {
                            applied |= self.apply_pushed(&frame);
                            self.broadcast(Event::Notification {
                                command: frame.command,
                                payload: frame.payload.clone(),
                            });
                        }
                        if applied {
                            let status = self.snapshot();
                            self.broadcast(Event::State { status });
                        }
                        if last_audio.elapsed() > audio_every {
                            last_audio = Instant::now();
                            let fresh = read_audio(self.snapshot().address.as_deref());
                            let changed = self
                                .state
                                .write()
                                .map(|mut slot| {
                                    let changed = slot.audio != fresh;
                                    slot.audio = fresh;
                                    changed
                                })
                                .unwrap_or(false);
                            if changed {
                                let status = self.snapshot();
                                self.broadcast(Event::State { status });
                            }
                        }
                        if last_track.elapsed() > track_every {
                            last_track = Instant::now();
                            let fresh = mpris::current();
                            let changed = self
                                .state
                                .write()
                                .map(|mut slot| {
                                    let changed = slot.now_playing != fresh;
                                    slot.now_playing = fresh;
                                    changed
                                })
                                .unwrap_or(false);
                            if changed {
                                let status = self.snapshot();
                                self.broadcast(Event::State { status });
                            }
                        }
                        if dirty || last_refresh.elapsed() > REFRESH_EVERY {
                            let status = Status {
                                connected: true,
                                channel: Some(active.channel),
                                error: None,
                                info: Info::read(active),
                                audio: read_audio(self.snapshot().address.as_deref()),
                                now_playing: mpris::current(),
                                address: self.snapshot().address,
                            };
                            self.publish(status);
                            last_refresh = Instant::now();
                            dirty = false;
                        }
                    }
                    Err(error) => {
                        eprintln!("m4d: link lost: {error}");
                        session = None;
                        self.disconnect(&error.to_string());
                    }
                },
            }
        }
    }

    fn handle_command(
        &self,
        session: &mut Option<Session>,
        request: Request,
        reply: Sender<Response>,
    ) -> (bool, bool) {
        if matches!(request, Request::Shutdown) {
            let _ = reply.send(Response::Ok);
            return (true, false);
        }
        let peer_change = matches!(
            request,
            Request::ConnectPeer { .. } | Request::DisconnectPeer { .. }
        );
        let optimistic = request.clone();
        let response = self.answer(session, request);
        if matches!(response, Response::Ok) {
            self.apply_local(&optimistic);
        }
        let _ = reply.send(response);
        (false, peer_change)
    }

    fn answer(&self, session: &mut Option<Session>, request: Request) -> Response {
        if let Err(message) = request
            .validate()
            .and_then(|()| validate_peer(&self.snapshot(), &request))
        {
            return Response::Error { message };
        }
        match request {
            Request::Status => Response::Status(self.snapshot()),
            Request::Watch | Request::Shutdown => Response::Ok,
            Request::Audio { set_profile } => {
                audio_state(set_profile, self.snapshot().address.as_deref())
            }
            other => match session.as_mut() {
                Some(active) => self.apply(active, other),
                None => Response::Error {
                    message: "headset is not connected".to_owned(),
                },
            },
        }
    }

    fn apply(&self, session: &mut Session, request: Request) -> Response {
        if let Request::Preset { name } = &request {
            let result = if matches!(name.as_str(), "neutral" | "off") {
                session.clear_eq()
            } else {
                match m4::preset(name) {
                    Some(preset) => session.apply_eq(preset.gains_db),
                    None => {
                        return Response::Error {
                            message: format!("unknown preset {name:?}; run `m4 preset list`"),
                        }
                    }
                }
            };
            return match result {
                Ok(()) => Response::Ok,
                Err(error) => Response::Error {
                    message: error.to_string(),
                },
            };
        }
        let outcome: Result<Option<String>, m4::Error> = (|| {
            Ok(match request {
                Request::Get { property } => Some(session.property(&property)?),
                Request::Anc { mode } => {
                    set_anc(session, mode)?;
                    None
                }
                Request::Transparency { level } => {
                    // Adaptive and Off both make the headset ignore the level, so
                    // moving the slider implies custom mode (same as the phone app).
                    let cached = self.snapshot();
                    if m4::transparency_needs_custom(&cached.info) {
                        session.set_anc_mode(3, 0)?;
                        session.set_anc_status(true)?;
                    }
                    session.set_transparency(level)?;
                    None
                }
                Request::EqGain { band, centibel } => {
                    session.set_eq_gain_and_enable(band, f32::from(centibel) / 10.0)?;
                    None
                }
                Request::BassBoost { enabled } => {
                    session.set_bass_boost(enabled)?;
                    None
                }
                Request::TouchLock { enabled } => {
                    session.set_touch_lock(enabled)?;
                    None
                }
                Request::ConnectPeer { index } => {
                    session.connect_paired(index)?;
                    None
                }
                Request::DisconnectPeer { index } => {
                    session.disconnect_paired(index)?;
                    None
                }
                Request::Status
                | Request::Watch
                | Request::Shutdown
                | Request::Audio { .. }
                | Request::Preset { .. } => None,
            })
        })();

        match outcome {
            Ok(Some(value)) => Response::Value { value },
            Ok(None) => Response::Ok,
            Err(error) => Response::Error {
                message: error.to_string(),
            },
        }
    }

    /// Reflects a successful write in the cache right away: the headset's own
    /// notification follows within ~25 ms and corrects us if it disagrees.
    fn apply_local(&self, request: &Request) {
        let Ok(mut status) = self.state.write() else {
            return;
        };
        let info = &mut status.info;
        match request {
            Request::Transparency { level } => {
                info.transparency = Some((*level).min(100));
                info.anc_status = Some(1);
                info.anc = Some("adaptive=0 anti-wind=0 comfort=0".to_owned());
            }
            Request::Anc { mode } => {
                info.anc_status = Some(u8::from(!matches!(mode, AncMode::Off)));
                info.anc = Some(match mode {
                    AncMode::Adaptive => "adaptive=1 anti-wind=0 comfort=0".to_owned(),
                    AncMode::Custom | AncMode::Off => "adaptive=0 anti-wind=0 comfort=0".to_owned(),
                });
            }
            Request::BassBoost { enabled } => info.bass_boost = Some(*enabled),
            Request::TouchLock { enabled } => info.touch_locked = Some(*enabled),
            Request::EqGain { band, centibel } => {
                if let Some(slot) = info.eq_gains_db.get_mut(usize::from(*band)) {
                    *slot = f32::from(*centibel) / 10.0;
                }
                info.audio_mode = Some("user EQ".to_owned());
            }
            Request::Preset { name } => {
                if matches!(name.as_str(), "neutral" | "off") {
                    info.audio_mode = Some("off (headset tuning)".to_owned());
                } else if let Some(preset) = m4::preset(name) {
                    info.eq_gains_db = preset.gains_db.to_vec();
                    info.audio_mode = Some("user EQ".to_owned());
                }
            }
            _ => return,
        }
        drop(status);
        let status = self.snapshot();
        self.broadcast(Event::State { status });
    }

    fn apply_pushed(&self, frame: &gaia::Frame) -> bool {
        match self.state.write() {
            Ok(mut status) => m4::apply_notification(&mut status.info, frame),
            Err(_) => false,
        }
    }

    fn snapshot(&self) -> Status {
        self.state
            .read()
            .map(|status| status.clone())
            .unwrap_or_default()
    }

    fn publish(&self, status: Status) {
        if let Ok(mut slot) = self.state.write() {
            *slot = status.clone();
        }
        self.broadcast(Event::State { status });
    }

    fn broadcast(&self, event: Event) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|sender| sender.send(event.clone()).is_ok());
        }
    }

    fn set_error(&self, reason: &str) {
        if let Ok(mut slot) = self.state.write() {
            slot.connected = false;
            slot.channel = None;
            slot.error = Some(reason.to_owned());
        }
    }

    fn disconnect(&self, reason: &str) {
        self.set_error(reason);
        self.broadcast(Event::Disconnected {
            reason: reason.to_owned(),
        });
        let status = self.snapshot();
        self.broadcast(Event::State { status });
    }
}

fn validate_peer(status: &Status, request: &Request) -> Result<(), String> {
    let index = match request {
        Request::ConnectPeer { index } | Request::DisconnectPeer { index } => *index,
        _ => return Ok(()),
    };
    if status.info.paired.iter().any(|peer| peer.index == index) {
        Ok(())
    } else {
        Err(format!(
            "paired device index {index} is not in the current snapshot"
        ))
    }
}

fn interval_from_env(name: &str, default: Duration) -> Duration {
    let Ok(value) = std::env::var(name) else {
        return default;
    };
    match parse_interval(&value) {
        Some(interval) => interval,
        None => {
            eprintln!("m4d: {name} expects positive seconds, got {value:?}; using {default:?}");
            default
        }
    }
}

fn parse_interval(value: &str) -> Option<Duration> {
    let seconds = value.trim().parse::<f64>().ok()?;
    Duration::try_from_secs_f64(seconds)
        .ok()
        .filter(|duration| !duration.is_zero())
}

fn read_audio(address: Option<&str>) -> Option<ipc::AudioState> {
    let card = audio::headset_for(address).ok().flatten()?;
    let sink = card.sink.as_ref();
    Some(ipc::AudioState {
        card: card.name.clone(),
        address: card.address.clone(),
        profile: card.active_profile.clone(),
        codec: card.codec().map(str::to_owned),
        sample_rate_hz: sink.and_then(|node| node.sample_rate_hz),
        channels: sink.and_then(|node| node.channels),
        format: sink.and_then(|node| node.format.clone()),
        state: sink.map(|node| node.state.clone()),
        is_headset: card.is_headset(),
        autoswitch: audio::autoswitch_headset_profile().ok().flatten(),
    })
}

fn audio_state(set_profile: Option<String>, address: Option<&str>) -> Response {
    let Some(card) = audio::headset_for(address).ok().flatten() else {
        return Response::Error {
            message: "no MOMENTUM 4 card in PipeWire".to_owned(),
        };
    };
    if let Some(profile) = set_profile {
        let Some(available) = card
            .profiles
            .iter()
            .find(|candidate| candidate.name == profile)
            .map(|candidate| candidate.available)
        else {
            return Response::Error {
                message: format!("unknown PipeWire profile {profile:?}"),
            };
        };
        if !available {
            return Response::Error {
                message: format!("PipeWire profile {profile:?} is unavailable"),
            };
        }
        if let Err(error) = audio::set_profile(&card.name, &profile) {
            return Response::Error {
                message: error.to_string(),
            };
        }
    }
    match read_audio(address) {
        Some(state) => Response::Audio(state),
        None => Response::Error {
            message: "audio card disappeared".to_owned(),
        },
    }
}

fn set_anc(session: &mut Session, mode: AncMode) -> Result<(), m4::Error> {
    // Order matters: the headset ignores ANC_Transparency while adaptive is on.
    match mode {
        AncMode::Adaptive => {
            session.set_anc_status(true)?;
            session.set_anc_mode(3, 1)
        }
        AncMode::Custom => {
            session.set_anc_mode(3, 0)?;
            session.set_anc_status(true)
        }
        AncMode::Off => {
            session.set_anc_mode(3, 0)?;
            session.set_anc_status(false)
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn connect() -> Result<Session, String> {
    Err("no GAIA transport on this platform yet (Linux only for now)".to_owned())
}

#[cfg(target_os = "linux")]
fn connect() -> Result<Session, String> {
    let device = rfcomm::bluez::find_headset()
        .map_err(|error| error.to_string())?
        .ok_or("no MOMENTUM 4 known to BlueZ")?;
    if !device.connected {
        return Err(format!("{} is not connected", device.name));
    }
    let address = m4::parse_address(&device.address)?;
    Session::open_resolved(address, m4::DEFAULT_TIMEOUT).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_intervals_accept_positive_seconds() {
        assert_eq!(parse_interval("2"), Some(Duration::from_secs(2)));
        assert_eq!(parse_interval(" 0.25 "), Some(Duration::from_millis(250)));
    }

    #[test]
    fn polling_intervals_reject_invalid_or_extreme_values() {
        for value in ["0", "-1", "NaN", "inf", "junk", "1e300"] {
            assert_eq!(parse_interval(value), None, "{value}");
        }
    }

    #[test]
    fn peer_requests_require_an_index_from_the_current_snapshot() {
        let mut status = Status::default();
        status.info.paired.push(m4::PairedEntry {
            index: 2,
            name: "phone".to_owned(),
            connected: false,
            priority: 0,
        });
        assert!(validate_peer(&status, &Request::ConnectPeer { index: 2 }).is_ok());
        assert!(validate_peer(&status, &Request::DisconnectPeer { index: 3 }).is_err());
        assert!(validate_peer(&status, &Request::Status).is_ok());
    }

    #[test]
    fn disconnect_broadcasts_an_offline_snapshot_to_watchers() {
        let (_commands, rx) = std::sync::mpsc::channel();
        let state = Arc::new(RwLock::new(Status {
            connected: true,
            channel: Some(15),
            error: None,
            ..Status::default()
        }));
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let (sender, events) = std::sync::mpsc::channel();
        subscribers.lock().unwrap().push(sender);
        let worker = Worker {
            rx,
            state,
            subscribers,
        };

        worker.disconnect("link lost");

        let events: Vec<Event> = events.try_iter().collect();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            Event::Disconnected { reason } if reason == "link lost"
        ));
        match &events[1] {
            Event::State { status } => {
                assert!(!status.connected);
                assert_eq!(status.channel, None);
                assert_eq!(status.error.as_deref(), Some("link lost"));
            }
            other => panic!("expected offline state, got {other:?}"),
        }
    }

    #[test]
    fn commands_interrupt_reconnect_backoff() {
        let (commands, rx) = std::sync::mpsc::channel();
        let state = Arc::new(RwLock::new(Status::default()));
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let worker_state = Arc::clone(&state);
        let handle = std::thread::spawn(move || {
            Worker {
                rx,
                state: worker_state,
                subscribers,
            }
            .run_with_connect(|| Err("test headset is offline".to_owned()));
        });

        let deadline = Instant::now() + Duration::from_secs(1);
        while state.read().unwrap().error.is_none() {
            assert!(Instant::now() < deadline, "worker never tried to connect");
            std::thread::sleep(Duration::from_millis(5));
        }

        let (reply, response) = std::sync::mpsc::channel();
        let started = Instant::now();
        commands
            .send(Command::Request {
                request: Request::Status,
                reply,
            })
            .unwrap();
        let status = response.recv_timeout(Duration::from_millis(250)).unwrap();
        assert!(started.elapsed() < Duration::from_millis(250));
        match status {
            Response::Status(status) => {
                assert!(!status.connected);
                assert_eq!(status.error.as_deref(), Some("test headset is offline"));
            }
            other => panic!("unexpected response: {other:?}"),
        }

        let (reply, response) = std::sync::mpsc::channel();
        commands
            .send(Command::Request {
                request: Request::Shutdown,
                reply,
            })
            .unwrap();
        assert_eq!(
            response.recv_timeout(Duration::from_millis(250)).unwrap(),
            Response::Ok
        );
        handle.join().unwrap();
    }
}
