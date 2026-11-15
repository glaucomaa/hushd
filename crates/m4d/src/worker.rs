use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::m4d_log;
use ipc::{AncMode, Event, Request, Response, Status};
use m4::{Info, Session};

const SUBSCRIBED_FEATURES: [u8; 6] = [3, 4, 2, 11, 12, 13];
const REFRESH_EVERY: Duration = Duration::from_secs(60);
const DEFAULT_AUDIO_EVERY: Duration = Duration::from_secs(2);
#[cfg(target_os = "linux")]
const DEFAULT_TRACK_EVERY: Duration = Duration::from_secs(2);
#[cfg(not(target_os = "linux"))]
const DEFAULT_TRACK_EVERY: Duration = Duration::from_secs(5);
const POLL_EVERY: Duration = Duration::from_millis(100);
const SYSTEM_CONNECTION_EVERY: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(15);
const LINK_LOSS_RETRY_DELAY: Duration = Duration::from_secs(3);

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
        if std::env::var_os("M4D_OFFLINE").is_some() {
            self.run_with_connect(|| Err("transport disabled".to_owned()));
        } else {
            self.run_with_connect(connect);
        }
    }

    fn run_with_connect<F>(&mut self, try_connect: F)
    where
        F: Fn() -> Result<Session, String> + Send + Sync + 'static,
    {
        self.run_with_connection_probe(try_connect, headset_connection, system_headset_connected);
    }

    #[cfg(test)]
    fn run_with_connect_and_control<F, C>(&mut self, try_connect: F, headset_control: C)
    where
        F: Fn() -> Result<Session, String> + Send + Sync + 'static,
        C: Fn(bool) -> Result<(), String>,
    {
        self.run_with_connection_probe(try_connect, headset_control, || Ok(true));
    }

    fn run_with_connection_probe<F, C, P>(
        &mut self,
        try_connect: F,
        headset_control: C,
        system_connected: P,
    ) where
        F: Fn() -> Result<Session, String> + Send + Sync + 'static,
        C: Fn(bool) -> Result<(), String>,
        P: Fn() -> Result<bool, String>,
    {
        let try_connect = Arc::new(try_connect);
        let audio_every = interval_from_env("M4_AUDIO_INTERVAL", DEFAULT_AUDIO_EVERY);
        let track_every = interval_from_env("M4_TRACK_INTERVAL", DEFAULT_TRACK_EVERY);
        let mut session: Option<Session> = None;
        let mut connecting: Option<Receiver<Result<Session, String>>> = None;
        let mut reconnect_at = None;
        let mut backoff = Duration::from_secs(1);
        let now = Instant::now();
        let mut last_refresh = now;
        let mut last_audio = now.checked_sub(audio_every).unwrap_or(now);
        let mut last_track = now.checked_sub(track_every).unwrap_or(now);
        let mut last_system_connection = now.checked_sub(SYSTEM_CONNECTION_EVERY).unwrap_or(now);
        let mut dirty = false;

        loop {
            let mut stop = false;
            while let Ok(Command::Request { request, reply }) = self.rx.try_recv() {
                let (should_stop, peer_change, connection_change) =
                    self.handle_command(&mut session, request, reply, &headset_control);
                if let Some(connect) = connection_change {
                    reconnect_at = (!connect).then(|| Instant::now() + LINK_LOSS_RETRY_DELAY);
                    if connect {
                        backoff = Duration::from_secs(1);
                    } else {
                        connecting = None;
                    }
                }
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

            if session.is_some() && last_system_connection.elapsed() >= SYSTEM_CONNECTION_EVERY {
                last_system_connection = Instant::now();
                if matches!(system_connected(), Ok(false)) {
                    m4d_log!("m4d: system Bluetooth disconnected");
                    session = None;
                    reconnect_at = Some(Instant::now() + LINK_LOSS_RETRY_DELAY);
                    self.disconnect("system Bluetooth disconnected");
                    continue;
                }
            }

            match session.as_mut() {
                None => {
                    if connecting.is_none()
                        && reconnect_at.is_none_or(|deadline| Instant::now() >= deadline)
                    {
                        self.set_error("connecting to MOMENTUM 4");
                        let (sender, receiver) = std::sync::mpsc::channel();
                        let connect = Arc::clone(&try_connect);
                        std::thread::spawn(move || {
                            let _ = sender.send(connect());
                        });
                        connecting = Some(receiver);
                        reconnect_at = None;
                    }
                    let outcome =
                        connecting
                            .as_ref()
                            .and_then(|pending| match pending.try_recv() {
                                Ok(outcome) => Some(outcome),
                                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                    Some(Err("connection worker stopped".to_owned()))
                                }
                            });
                    if let Some(outcome) = outcome {
                        connecting = None;
                        match outcome {
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
                                m4d_log!("m4d: connected on rfcomm channel {channel}");
                                self.broadcast(Event::Connected { channel });
                                session = Some(fresh);
                                backoff = Duration::from_secs(1);
                                last_refresh = Instant::now();
                                dirty = false;
                            }
                            Err(reason) => {
                                m4d_log!("m4d: {reason}");
                                self.set_error(&reason);
                                reconnect_at = Some(Instant::now() + backoff);
                                backoff = (backoff * 2).min(MAX_BACKOFF);
                            }
                        }
                    }
                    match self.rx.recv_timeout(POLL_EVERY) {
                        Ok(Command::Request { request, reply }) => {
                            let (stop, peer_change, connection_change) =
                                self.handle_command(&mut session, request, reply, &headset_control);
                            if let Some(connect) = connection_change {
                                reconnect_at =
                                    (!connect).then(|| Instant::now() + LINK_LOSS_RETRY_DELAY);
                                if connect {
                                    backoff = Duration::from_secs(1);
                                } else {
                                    connecting = None;
                                }
                            }
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
                }
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
                        m4d_log!("m4d: link lost: {error}");
                        session = None;
                        reconnect_at = Some(Instant::now() + LINK_LOSS_RETRY_DELAY);
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
        headset_control: &dyn Fn(bool) -> Result<(), String>,
    ) -> (bool, bool, Option<bool>) {
        if matches!(request, Request::Shutdown) {
            let _ = reply.send(Response::Ok);
            return (true, false, None);
        }
        let peer_change = matches!(
            request,
            Request::ConnectPeer { .. }
                | Request::DisconnectPeer { .. }
                | Request::HeadsetConnect { .. }
        );
        let optimistic = request.clone();
        let response = self.answer(session, request, headset_control);
        let connection_change = successful_headset_connection(&optimistic, &response);
        if matches!(response, Response::Ok) {
            self.apply_local(&optimistic);
        }
        let _ = reply.send(response);
        (false, peer_change, connection_change)
    }

    fn answer(
        &self,
        session: &mut Option<Session>,
        request: Request,
        headset_control: &dyn Fn(bool) -> Result<(), String>,
    ) -> Response {
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
            Request::HeadsetConnect { connect } => match headset_control(connect) {
                Ok(()) => {
                    if !connect {
                        *session = None;
                    }
                    Response::Ok
                }
                Err(message) => Response::Error { message },
            },
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
                    // Transparency changes imply custom mode.
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
                | Request::HeadsetConnect { .. }
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
            Request::HeadsetConnect { connect } => {
                status.connected = false;
                status.channel = None;
                status.error = Some(if *connect {
                    "connecting to MOMENTUM 4".to_owned()
                } else {
                    "headset disconnected".to_owned()
                });
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
            slot.audio = None;
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

fn successful_headset_connection(request: &Request, response: &Response) -> Option<bool> {
    match (request, response) {
        (Request::HeadsetConnect { connect }, Response::Ok) => Some(*connect),
        _ => None,
    }
}

fn validate_peer(status: &Status, request: &Request) -> Result<(), String> {
    let index = match request {
        Request::ConnectPeer { index } | Request::DisconnectPeer { index } => *index,
        _ => return Ok(()),
    };
    let Some(device) = status.info.paired.iter().find(|peer| peer.index == index) else {
        return Err(format!(
            "paired device index {index} is not in the current snapshot"
        ));
    };
    let connected = status
        .info
        .paired
        .iter()
        .filter(|peer| peer.connected)
        .count();
    if matches!(request, Request::ConnectPeer { .. }) && !device.connected && connected >= 2 {
        return Err(
            "both multipoint slots are occupied; disconnect a connected host first".to_owned(),
        );
    }
    Ok(())
}

fn interval_from_env(name: &str, default: Duration) -> Duration {
    let Ok(value) = std::env::var(name) else {
        return default;
    };
    match parse_interval(&value) {
        Some(interval) => interval,
        None => {
            m4d_log!("m4d: {name} expects positive seconds, got {value:?}; using {default:?}");
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

#[cfg(target_os = "linux")]
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

#[cfg(not(target_os = "linux"))]
fn read_audio(_address: Option<&str>) -> Option<ipc::AudioState> {
    None
}

#[cfg(target_os = "linux")]
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

#[cfg(not(target_os = "linux"))]
fn audio_state(_set_profile: Option<String>, _address: Option<&str>) -> Response {
    Response::Error {
        message: "audio profiles are managed by macOS; codec and sample rate are available in `m4 status`".to_owned(),
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn headset_connection(connect: bool) -> Result<(), String> {
    rfcomm::discovery::set_headset_connected(connect).map_err(|error| error.to_string())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn headset_connection(_connect: bool) -> Result<(), String> {
    Err("system headset connection control is unavailable on this platform".to_owned())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn system_headset_connected() -> Result<bool, String> {
    let device = rfcomm::discovery::find_headset()
        .map_err(|error| error.to_string())?
        .ok_or("no paired MOMENTUM 4 found")?;
    Ok(device.connected)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn system_headset_connected() -> Result<bool, String> {
    Ok(true)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn connect() -> Result<Session, String> {
    let device = rfcomm::discovery::find_headset()
        .map_err(|error| error.to_string())?
        .ok_or("no paired MOMENTUM 4 found")?;
    if !device.connected {
        return Err(format!("{} is not connected", device.name));
    }
    let address = m4::parse_address(&device.address)?;
    Session::open_resolved(address, m4::DEFAULT_TIMEOUT).map_err(|error| error.to_string())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn connect() -> Result<Session, String> {
    Err("no GAIA transport on this platform".to_owned())
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
    fn connect_peer_rejects_a_third_active_host() {
        let mut status = Status::default();
        for (index, connected) in [(0, true), (1, true), (2, false)] {
            status.info.paired.push(m4::PairedEntry {
                index,
                name: format!("device {index}"),
                connected,
                priority: 0,
            });
        }

        let error = validate_peer(&status, &Request::ConnectPeer { index: 2 })
            .expect_err("third active host must be rejected");
        assert!(error.contains("both multipoint slots"));
        assert!(validate_peer(&status, &Request::DisconnectPeer { index: 0 }).is_ok());
        assert!(validate_peer(&status, &Request::ConnectPeer { index: 0 }).is_ok());
    }

    #[test]
    fn headset_connection_state_changes_only_after_success() {
        let disconnect = Request::HeadsetConnect { connect: false };
        let failure = Response::Error {
            message: "BlueZ rejected disconnect".to_owned(),
        };
        assert_eq!(successful_headset_connection(&disconnect, &failure), None);
        assert_eq!(
            successful_headset_connection(&disconnect, &Response::Ok),
            Some(false)
        );

        let connect = Request::HeadsetConnect { connect: true };
        assert_eq!(
            successful_headset_connection(&connect, &Response::Ok),
            Some(true)
        );
        assert_eq!(
            successful_headset_connection(&Request::Status, &Response::Ok),
            None
        );
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
        while state.read().unwrap().error.as_deref() != Some("test headset is offline") {
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

    #[test]
    fn external_reconnect_is_not_blocked_after_manual_disconnect() {
        let (commands, rx) = std::sync::mpsc::channel();
        let state = Arc::new(RwLock::new(Status::default()));
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_state = Arc::clone(&state);
        let worker_attempts = Arc::clone(&attempts);
        let handle = std::thread::spawn(move || {
            Worker {
                rx,
                state: worker_state,
                subscribers,
            }
            .run_with_connect_and_control(
                move || {
                    worker_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Err("test headset is offline".to_owned())
                },
                |_| Ok(()),
            );
        });

        let (reply, response) = std::sync::mpsc::channel();
        commands
            .send(Command::Request {
                request: Request::HeadsetConnect { connect: false },
                reply,
            })
            .unwrap();
        assert_eq!(
            response.recv_timeout(Duration::from_millis(250)).unwrap(),
            Response::Ok
        );
        let baseline = attempts.load(std::sync::atomic::Ordering::SeqCst);
        let deadline = Instant::now() + LINK_LOSS_RETRY_DELAY + Duration::from_secs(2);
        while attempts.load(std::sync::atomic::Ordering::SeqCst) <= baseline {
            assert!(
                Instant::now() < deadline,
                "manual disconnect permanently blocked connection attempts"
            );
            std::thread::sleep(Duration::from_millis(20));
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

    #[test]
    fn system_disconnect_closes_the_gaia_session() {
        struct IdleTransport;

        impl m4::Transport for IdleTransport {
            fn send(&self, _bytes: &[u8]) -> std::io::Result<()> {
                Ok(())
            }

            fn recv(
                &self,
                _buffer: &mut [u8],
                timeout: Duration,
            ) -> std::io::Result<Option<usize>> {
                if timeout == Duration::ZERO || timeout <= POLL_EVERY {
                    Ok(None)
                } else {
                    Ok(Some(0))
                }
            }
        }

        let (commands, rx) = std::sync::mpsc::channel();
        let state = Arc::new(RwLock::new(Status::default()));
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let system_connected = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let worker_state = Arc::clone(&state);
        let worker_connected = Arc::clone(&system_connected);
        let handle = std::thread::spawn(move || {
            Worker {
                rx,
                state: worker_state,
                subscribers,
            }
            .run_with_connection_probe(
                || Ok(Session::from_transport(Box::new(IdleTransport), 1)),
                |_| Ok(()),
                move || Ok(worker_connected.load(std::sync::atomic::Ordering::SeqCst)),
            );
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        while !state.read().unwrap().connected {
            assert!(Instant::now() < deadline, "worker never connected");
            std::thread::sleep(Duration::from_millis(10));
        }
        state.write().unwrap().audio = Some(ipc::AudioState::default());
        system_connected.store(false, std::sync::atomic::Ordering::SeqCst);

        while state.read().unwrap().connected {
            assert!(
                Instant::now() < deadline,
                "system disconnect left the GAIA session active"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let status = state.read().unwrap().clone();
        assert_eq!(
            status.error.as_deref(),
            Some("system Bluetooth disconnected")
        );
        assert_eq!(status.audio, None);

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

    #[test]
    fn slow_connect_does_not_block_commands() {
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
            .run_with_connect(|| {
                std::thread::sleep(Duration::from_secs(2));
                Err("slow test connect failed".to_owned())
            });
        });

        let deadline = Instant::now() + Duration::from_secs(1);
        while state.read().unwrap().error.is_none() {
            assert!(Instant::now() < deadline, "worker never started connecting");
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
        assert!(matches!(status, Response::Status(_)));

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
