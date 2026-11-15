use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::layout::{Position, Rect};

use ipc::{find_peer, peer_key, peer_request, AncMode, Event, PeerKey, Request, Response, Status};

use crate::Backend;

const LOG_LIMIT: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppLayout {
    Instrument,
    Minimal,
}

impl AppLayout {
    pub fn toggled(self) -> Self {
        match self {
            Self::Instrument => Self::Minimal,
            Self::Minimal => Self::Instrument,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Overview,
    Noise,
    Sound,
    Devices,
    Diagnostics,
}

impl Screen {
    pub fn next(self) -> Self {
        match self {
            Self::Overview => Self::Noise,
            Self::Noise => Self::Sound,
            Self::Sound => Self::Devices,
            Self::Devices => Self::Diagnostics,
            Self::Diagnostics => Self::Overview,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Overview => Self::Diagnostics,
            Self::Noise => Self::Overview,
            Self::Sound => Self::Noise,
            Self::Devices => Self::Sound,
            Self::Diagnostics => Self::Devices,
        }
    }

    pub fn from_number(number: u8) -> Option<Self> {
        match number {
            1 => Some(Self::Overview),
            2 => Some(Self::Noise),
            3 => Some(Self::Sound),
            4 => Some(Self::Devices),
            5 => Some(Self::Diagnostics),
            _ => None,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Noise => "Noise",
            Self::Sound => "Sound",
            Self::Devices => "Devices",
            Self::Diagnostics => "Diagnostics",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Tab,
    BackTab,
    Number(u8),
    Char(char),
    Enter,
    Esc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceAction {
    Connect,
    Disconnect,
}

#[derive(Debug, Clone)]
pub struct Confirmation {
    pub action: DeviceAction,
    pub index: u8,
    pub name: String,
    pub own: bool,
}

const DEBOUNCE: Duration = Duration::from_millis(80);

#[derive(Default, Clone)]
pub struct Hitboxes {
    pub eq: Option<Rect>,
    pub list: Option<Rect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mouse {
    ScrollUp,
    ScrollDown,
    Click { column: u16, row: u16 },
}

pub struct App {
    pub screen: Screen,
    pub hitboxes: RefCell<Hitboxes>,
    pub show_presets: bool,
    pub preset_selected: usize,
    pub debounce: Duration,
    pending: Option<(Request, Instant)>,
    pub layout: AppLayout,
    pub selected: usize,
    pub status: Status,
    pub should_quit: bool,
    pub show_help: bool,
    pub message: Option<String>,
    pub daemon_error: Option<String>,
    pub confirmation: Option<Confirmation>,
    pub notifications: VecDeque<String>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn with_layout(layout: AppLayout) -> Self {
        let mut app = Self::new();
        app.layout = layout;
        app
    }

    pub fn new() -> Self {
        Self {
            screen: Screen::Overview,
            hitboxes: RefCell::new(Hitboxes::default()),
            show_presets: false,
            preset_selected: 0,
            debounce: DEBOUNCE,
            pending: None,
            layout: AppLayout::Instrument,
            selected: 0,
            status: Status::default(),
            should_quit: false,
            show_help: false,
            message: None,
            daemon_error: None,
            confirmation: None,
            notifications: VecDeque::new(),
        }
    }

    pub fn refresh(&mut self, backend: &mut impl Backend) {
        match backend.status() {
            Ok(status) => {
                self.update_status(status);
                self.daemon_error = None;
            }
            Err(error) => {
                self.status.connected = false;
                self.status.error = Some(error.clone());
                self.daemon_error = Some(error);
            }
        }
    }

    pub fn apply_event(&mut self, event: Event) {
        match event {
            Event::Connected { channel } => {
                self.push_log(format!("connected: RFCOMM channel {channel}"))
            }
            Event::Disconnected { reason } => {
                self.status.connected = false;
                self.push_log(format!("disconnected: {reason}"));
            }
            Event::State { status } => self.update_status(status),
            Event::Notification { command, payload } => {
                let payload = payload
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                self.push_log(format!("{command:#06x} {payload}"));
            }
        }
    }

    fn update_status(&mut self, status: Status) {
        let selected = self.selected_peer_key();
        self.status = status;
        if let Some(key) = selected {
            if let Some(index) = find_peer(&self.status, &key) {
                if let Some(row) = self
                    .status
                    .info
                    .paired
                    .iter()
                    .position(|device| device.index == index)
                {
                    self.selected = row;
                }
            }
        }
        self.clamp_selection();
    }

    fn selected_peer_key(&self) -> Option<PeerKey> {
        if self.screen != Screen::Devices {
            return None;
        }
        let device = self.status.info.paired.get(self.selected)?;
        peer_key(&self.status, device.index)
    }

    pub fn handle_key(&mut self, key: Key, backend: &mut impl Backend) {
        if self.show_presets {
            match key {
                Key::Esc | Key::Char('p') => self.show_presets = false,
                Key::Up => self.preset_selected = self.preset_selected.saturating_sub(1),
                Key::Down => {
                    self.preset_selected = (self.preset_selected + 1).min(preset_names().len() - 1)
                }
                Key::Enter => {
                    let name = preset_names()[self.preset_selected].to_owned();
                    self.show_presets = false;
                    self.send(Request::Preset { name }, backend);
                }
                _ => {}
            }
            return;
        }
        if matches!(key, Key::Esc | Key::Char('q')) {
            self.should_quit = true;
            return;
        }
        if key == Key::Char('?') {
            self.show_help = !self.show_help;
            return;
        }
        if self.confirmation.is_some() {
            match key {
                Key::Char('y') => self.confirm(backend),
                Key::Char('n') => self.confirmation = None,
                _ => {}
            }
            return;
        }
        if self.screen == Screen::Sound && !self.show_help {
            match key {
                // The EQ is a vertical bar chart: left/right walks the bands,
                // up/down changes the height, the way the graph reads.
                Key::Left => {
                    self.move_sound_selection(-1);
                    self.clamp_selection();
                    return;
                }
                Key::Right => {
                    self.move_sound_selection(1);
                    self.clamp_selection();
                    return;
                }
                Key::Up => {
                    self.change_sound(1, backend);
                    self.clamp_selection();
                    return;
                }
                Key::Down => {
                    self.change_sound(-1, backend);
                    self.clamp_selection();
                    return;
                }
                _ => {}
            }
        }
        match key {
            Key::Tab => self.screen = self.screen.next(),
            Key::BackTab => self.screen = self.screen.previous(),
            Key::Number(number) => {
                if let Some(screen) = Screen::from_number(number) {
                    self.screen = screen;
                }
            }
            Key::Up => self.move_selection(-1),
            Key::Down => self.move_selection(1),

            Key::Char('r') => self.refresh(backend),
            Key::Char('m') if !self.show_help => self.layout = self.layout.toggled(),
            Key::Char('p') if self.screen == Screen::Sound && !self.show_help => {
                self.show_presets = true;
                self.preset_selected = 0;
            }
            Key::Char('a')
                if self
                    .status
                    .audio
                    .as_ref()
                    .is_some_and(|audio| audio.is_headset) =>
            {
                self.send(
                    Request::Audio {
                        set_profile: Some("a2dp-sink".to_owned()),
                    },
                    backend,
                );
            }
            Key::Left if !self.show_help => self.change(-1, backend),
            Key::Right if !self.show_help => self.change(1, backend),
            Key::Char('c') if !self.show_help => self.confirm_device(DeviceAction::Connect),
            Key::Char('d') if !self.show_help => self.confirm_device(DeviceAction::Disconnect),
            _ => {}
        }
        self.clamp_selection();
    }

    pub fn anc_mode(&self) -> AncMode {
        if self.status.info.anc_status == Some(0) {
            return AncMode::Off;
        }
        if self
            .status
            .info
            .anc
            .as_deref()
            .is_some_and(|anc| anc.contains("adaptive=1"))
        {
            AncMode::Adaptive
        } else {
            AncMode::Custom
        }
    }

    fn move_selection(&mut self, direction: i32) {
        let count = self.row_count();
        if count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as i32 + direction).rem_euclid(count as i32) as usize;
    }

    fn clamp_selection(&mut self) {
        let count = self.row_count();
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }

    fn row_count(&self) -> usize {
        match self.screen {
            Screen::Noise => 3,
            Screen::Sound => 6,
            Screen::Devices => self.status.info.paired.len(),
            Screen::Overview | Screen::Diagnostics => 0,
        }
    }

    pub fn handle_mouse(&mut self, mouse: Mouse, backend: &mut impl Backend) {
        let hitboxes = self.hitboxes.borrow().clone();
        match mouse {
            Mouse::ScrollUp | Mouse::ScrollDown => {
                let direction = if matches!(mouse, Mouse::ScrollUp) {
                    1
                } else {
                    -1
                };
                match self.screen {
                    Screen::Sound => self.change_sound(direction, backend),
                    Screen::Noise => self.change_transparency(direction, backend),
                    Screen::Devices => self.move_selection(direction),
                    _ => {}
                }
            }
            Mouse::Click { column, row } => match self.screen {
                Screen::Sound => {
                    if let Some(eq) = hitboxes.eq {
                        if eq.contains(Position::new(column, row)) {
                            self.select_band_at(eq, column);
                        }
                    }
                }
                Screen::Devices => {
                    if let Some(list) = hitboxes.list {
                        if list.contains(Position::new(column, row)) {
                            let index = usize::from(row.saturating_sub(list.y));
                            if index < self.status.info.paired.len() {
                                self.selected = index;
                            }
                        }
                    }
                }
                _ => {}
            },
        }
        self.clamp_selection();
    }

    fn select_band_at(&mut self, area: Rect, column: u16) {
        if area.width == 0 {
            return;
        }
        let offset = u32::from(column.saturating_sub(area.x));
        let band = (offset * 5 / u32::from(area.width)).min(4);
        self.selected = band as usize;
    }

    fn change(&mut self, direction: i32, backend: &mut impl Backend) {
        if self.screen == Screen::Noise {
            match self.selected {
                0 => self.change_anc(direction, backend),
                1 => self.change_transparency(direction, backend),
                _ => {}
            }
        }
    }

    fn move_sound_selection(&mut self, direction: i32) {
        self.selected = (self.selected as i32 + direction).rem_euclid(6) as usize;
    }

    fn change_sound(&mut self, direction: i32, backend: &mut impl Backend) {
        if self.selected < 5 {
            self.change_eq(self.selected, direction, backend);
        } else {
            let enabled = !self.status.info.bass_boost.unwrap_or(false);
            self.send(Request::BassBoost { enabled }, backend);
        }
    }

    fn change_anc(&mut self, direction: i32, backend: &mut impl Backend) {
        let modes = [AncMode::Adaptive, AncMode::Custom, AncMode::Off];
        let current = self.anc_mode();
        let index = modes.iter().position(|mode| *mode == current).unwrap_or(0);
        let next = (index as i32 + direction).rem_euclid(modes.len() as i32) as usize;
        self.send(Request::Anc { mode: modes[next] }, backend);
    }

    fn change_transparency(&mut self, direction: i32, backend: &mut impl Backend) {
        let current = i16::from(self.status.info.transparency.unwrap_or(0));
        let level = (current + direction as i16 * 5).clamp(0, 100) as u8;
        self.queue(Request::Transparency { level }, backend);
    }

    fn change_eq(&mut self, index: usize, direction: i32, backend: &mut impl Backend) {
        let band = index as u8;
        let current = self
            .status
            .info
            .eq_gains_db
            .get(index)
            .copied()
            .unwrap_or(0.0);
        let centibel = ((current * 10.0).round() as i16 + direction as i16 * 5).clamp(-60, 60);
        self.queue(Request::EqGain { band, centibel }, backend);
    }

    fn confirm_device(&mut self, action: DeviceAction) {
        if self.screen != Screen::Devices {
            return;
        }
        if let Some(device) = self.status.info.paired.get(self.selected) {
            self.confirmation = Some(Confirmation {
                action,
                index: device.index,
                name: device.name.clone(),
                own: self.status.info.own_index == Some(device.index),
            });
        }
    }

    fn confirm(&mut self, backend: &mut impl Backend) {
        let Some(confirmation) = self.confirmation.take() else {
            return;
        };
        let key = PeerKey {
            name: confirmation.name,
            own: confirmation.own,
        };
        let Some(index) = find_peer(&self.status, &key) else {
            self.message = Some("device list changed; select the device again".to_owned());
            return;
        };
        let connect = matches!(confirmation.action, DeviceAction::Connect);
        if let Some(request) = peer_request(&self.status, index, connect) {
            self.send(request, backend);
        }
    }

    /// Sliders feel instant because the value moves locally first; the write is
    /// coalesced so a held arrow key does not flood the 25 ms RFCOMM link.
    fn queue(&mut self, request: Request, backend: &mut impl Backend) {
        if !self.status.connected {
            self.message = Some("headset not connected; write blocked".to_owned());
            return;
        }
        let _ = backend;
        self.apply_local(&request);
        self.pending = Some((request, Instant::now()));
    }

    fn apply_local(&mut self, request: &Request) {
        match request {
            Request::Transparency { level } => {
                self.status.info.transparency = Some((*level).min(100));
                self.status.info.anc_status = Some(1);
                self.status.info.anc = Some("adaptive=0 anti-wind=0 comfort=0".to_owned());
            }
            Request::EqGain { band, centibel } => {
                if let Some(slot) = self.status.info.eq_gains_db.get_mut(usize::from(*band)) {
                    *slot = f32::from(*centibel) / 10.0;
                }
            }
            _ => {}
        }
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn tick(&mut self, backend: &mut impl Backend) {
        let Some((_, at)) = self.pending.as_ref() else {
            return;
        };
        if at.elapsed() < self.debounce {
            return;
        }
        self.flush(backend);
    }

    pub fn flush(&mut self, backend: &mut impl Backend) {
        if let Some((request, _)) = self.pending.take() {
            self.send(request, backend);
        }
    }

    fn send(&mut self, request: Request, backend: &mut impl Backend) {
        if !self.status.connected {
            self.message = Some("headset not connected; write blocked".to_owned());
            return;
        }
        match backend.dispatch(request) {
            Ok(Response::Error { message }) => self.message = Some(message),
            Ok(_) => {
                self.message = None;
                self.refresh(backend);
            }
            Err(error) => self.message = Some(error),
        }
    }

    fn push_log(&mut self, message: String) {
        if self.notifications.len() == LOG_LIMIT {
            self.notifications.pop_front();
        }
        self.notifications.push_back(message);
    }
}

pub fn preset_names() -> Vec<&'static str> {
    let mut names = vec!["neutral"];
    names.extend(m4::PRESETS.iter().map(|preset| preset.name));
    names
}

pub fn preset_gains(name: &str) -> Option<[f32; 5]> {
    m4::preset(name).map(|preset| preset.gains_db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockBackend;

    fn app_and_backend() -> (App, MockBackend) {
        let mut app = App::new();
        let mut backend = MockBackend::new();
        app.refresh(&mut backend);
        (app, backend)
    }

    #[test]
    fn tab_cycles_screens_in_both_directions() {
        let (mut app, mut backend) = app_and_backend();
        assert_eq!(app.screen, Screen::Overview);
        for expected in [
            Screen::Noise,
            Screen::Sound,
            Screen::Devices,
            Screen::Diagnostics,
            Screen::Overview,
        ] {
            app.handle_key(Key::Tab, &mut backend);
            assert_eq!(app.screen, expected);
        }
        app.handle_key(Key::BackTab, &mut backend);
        assert_eq!(app.screen, Screen::Diagnostics);
        app.handle_key(Key::Number(2), &mut backend);
        assert_eq!(app.screen, Screen::Noise);
    }

    #[test]
    fn transparency_moves_locally_then_writes_once() {
        let (mut app, mut backend) = app_and_backend();
        app.screen = Screen::Noise;
        app.selected = 1;
        app.handle_key(Key::Right, &mut backend);
        app.handle_key(Key::Right, &mut backend);
        assert_eq!(
            app.status.info.transparency,
            Some(60),
            "bar reacts immediately"
        );
        assert!(backend.requests.is_empty(), "writes are debounced");

        app.flush(&mut backend);
        assert_eq!(backend.requests, vec![Request::Transparency { level: 60 }]);
    }

    #[test]
    fn transparency_clamps_at_both_ends() {
        let (mut app, mut backend) = app_and_backend();
        app.screen = Screen::Noise;
        app.selected = 1;
        for _ in 0..30 {
            app.handle_key(Key::Right, &mut backend);
        }
        assert_eq!(app.status.info.transparency, Some(100));
        app.flush(&mut backend);
        assert_eq!(
            backend.requests.last(),
            Some(&Request::Transparency { level: 100 })
        );

        for _ in 0..30 {
            app.handle_key(Key::Left, &mut backend);
        }
        assert_eq!(app.status.info.transparency, Some(0));
        app.flush(&mut backend);
        assert_eq!(
            backend.requests.last(),
            Some(&Request::Transparency { level: 0 })
        );
    }

    #[test]
    fn sound_arrows_are_spatial() {
        let (mut app, mut backend) = app_and_backend();
        app.screen = Screen::Sound;
        app.selected = 0;

        app.handle_key(Key::Up, &mut backend);
        assert_eq!(app.status.info.eq_gains_db[0], 0.5, "up raises the bar");
        app.flush(&mut backend);
        assert_eq!(
            backend.requests.last(),
            Some(&Request::EqGain {
                band: 0,
                centibel: 5
            })
        );

        app.handle_key(Key::Down, &mut backend);
        app.handle_key(Key::Down, &mut backend);
        assert_eq!(app.status.info.eq_gains_db[0], -0.5);
        app.flush(&mut backend);
        assert_eq!(
            backend.requests.last(),
            Some(&Request::EqGain {
                band: 0,
                centibel: -5
            })
        );

        for _ in 0..40 {
            app.handle_key(Key::Up, &mut backend);
        }
        assert_eq!(app.status.info.eq_gains_db[0], 6.0, "clamped at +6 dB");
    }

    #[test]
    fn sound_left_right_picks_the_band() {
        let (mut app, mut backend) = app_and_backend();
        app.screen = Screen::Sound;
        app.selected = 0;

        app.handle_key(Key::Right, &mut backend);
        assert_eq!(app.selected, 1);
        app.handle_key(Key::Left, &mut backend);
        app.handle_key(Key::Left, &mut backend);
        assert_eq!(app.selected, 5, "wraps onto bass boost");
        assert!(backend.requests.is_empty(), "navigation must not write");

        app.handle_key(Key::Up, &mut backend);
        assert_eq!(backend.requests, vec![Request::BassBoost { enabled: true }]);
    }

    #[test]
    fn tick_only_flushes_after_the_debounce() {
        let (mut app, mut backend) = app_and_backend();
        app.debounce = Duration::from_millis(40);
        app.screen = Screen::Noise;
        app.selected = 1;
        app.handle_key(Key::Right, &mut backend);
        app.tick(&mut backend);
        assert!(backend.requests.is_empty(), "too early");
        std::thread::sleep(Duration::from_millis(50));
        app.tick(&mut backend);
        assert_eq!(backend.requests, vec![Request::Transparency { level: 55 }]);
        assert!(!app.has_pending());
    }

    #[test]
    fn preset_picker_applies_a_named_preset() {
        let (mut app, mut backend) = app_and_backend();
        app.screen = Screen::Sound;

        app.handle_key(Key::Char('p'), &mut backend);
        assert!(app.show_presets);
        assert_eq!(app.preset_selected, 0);

        app.handle_key(Key::Down, &mut backend);
        app.handle_key(Key::Down, &mut backend);
        app.handle_key(Key::Down, &mut backend);
        app.handle_key(Key::Enter, &mut backend);
        assert!(!app.show_presets);
        assert_eq!(
            backend.requests.last(),
            Some(&Request::Preset {
                name: "dance".to_owned()
            })
        );
    }

    #[test]
    fn preset_picker_escape_closes_without_quitting() {
        let (mut app, mut backend) = app_and_backend();
        app.screen = Screen::Sound;
        app.handle_key(Key::Char('p'), &mut backend);
        app.handle_key(Key::Esc, &mut backend);
        assert!(!app.show_presets);
        assert!(!app.should_quit);

        app.screen = Screen::Overview;
        app.handle_key(Key::Char('p'), &mut backend);
        assert!(!app.show_presets, "the picker only belongs to Sound");
    }

    #[test]
    fn wheel_changes_transparency_on_noise() {
        let (mut app, mut backend) = app_and_backend();
        app.screen = Screen::Noise;
        app.handle_mouse(Mouse::ScrollUp, &mut backend);
        assert_eq!(app.status.info.transparency, Some(55));
        app.flush(&mut backend);
        assert_eq!(
            backend.requests.last(),
            Some(&Request::Transparency { level: 55 })
        );
    }

    #[test]
    fn click_picks_an_eq_band_and_wheel_changes_it() {
        let (mut app, mut backend) = app_and_backend();
        app.screen = Screen::Sound;
        app.hitboxes.borrow_mut().eq = Some(Rect::new(0, 0, 50, 10));

        app.handle_mouse(Mouse::Click { column: 25, row: 5 }, &mut backend);
        assert_eq!(app.selected, 2);
        assert!(backend.requests.is_empty(), "click only selects");

        app.handle_mouse(Mouse::ScrollUp, &mut backend);
        assert_eq!(app.status.info.eq_gains_db[2], 0.5);
        app.flush(&mut backend);
        assert_eq!(
            backend.requests.last(),
            Some(&Request::EqGain {
                band: 2,
                centibel: 5
            })
        );
    }

    #[test]
    fn click_picks_a_paired_device_row() {
        let (mut app, mut backend) = app_and_backend();
        app.screen = Screen::Devices;
        let device = m4::PairedEntry {
            index: 2,
            name: "WIN".to_owned(),
            connected: false,
            priority: 2,
        };
        app.status.info.paired = vec![device.clone(), device];
        app.hitboxes.borrow_mut().list = Some(Rect::new(0, 0, 40, 10));

        app.handle_mouse(Mouse::Click { column: 5, row: 1 }, &mut backend);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn paired_selection_and_confirmation_follow_device_across_reordering() {
        let (mut app, mut backend) = app_and_backend();
        let device = |index: u8, name: &str| m4::PairedEntry {
            index,
            name: name.to_owned(),
            connected: true,
            priority: 0,
        };
        let mut status = backend.status().unwrap();
        status.info.own_index = Some(0);
        status.info.paired = vec![device(0, "linux"), device(1, "mac")];
        backend.set_status(status.clone());
        app.refresh(&mut backend);
        app.screen = Screen::Devices;
        app.selected = 1;
        app.confirm_device(DeviceAction::Disconnect);

        status.info.own_index = Some(1);
        status.info.paired = vec![device(0, "mac"), device(1, "linux")];
        backend.set_status(status.clone());
        app.apply_event(Event::State { status });
        assert_eq!(app.selected, 0);

        app.confirm(&mut backend);
        assert_eq!(
            backend.requests.last(),
            Some(&Request::DisconnectPeer { index: 0 })
        );
    }

    #[test]
    fn own_device_actions_use_the_system_bluetooth_backend() {
        let (mut app, mut backend) = app_and_backend();
        let mut status = backend.status().expect("mock status");
        status.info.own_index = Some(1);
        status.info.paired = (1..=2)
            .map(|index| m4::PairedEntry {
                index,
                name: "device".to_owned(),
                connected: true,
                priority: 0,
            })
            .collect();
        backend.set_status(status);
        app.refresh(&mut backend);

        for (index, action, expected) in [
            (
                1,
                DeviceAction::Disconnect,
                Request::HeadsetConnect { connect: false },
            ),
            (2, DeviceAction::Connect, Request::ConnectPeer { index: 2 }),
            (
                1,
                DeviceAction::Connect,
                Request::HeadsetConnect { connect: true },
            ),
        ] {
            app.confirmation = Some(Confirmation {
                action,
                index,
                name: "device".to_owned(),
                own: index == 1,
            });
            app.confirm(&mut backend);
            assert_eq!(backend.requests.last(), Some(&expected));
        }
    }

    #[test]
    fn selecting_anc_off_dispatches_off() {
        let (mut app, mut backend) = app_and_backend();
        app.screen = Screen::Noise;
        app.selected = 0;
        app.handle_key(Key::Right, &mut backend);
        assert_eq!(backend.requests, vec![Request::Anc { mode: AncMode::Off }]);
        assert_eq!(app.anc_mode(), AncMode::Off);
    }

    #[test]
    fn disconnected_headset_blocks_writes() {
        let (mut app, mut backend) = app_and_backend();
        let mut status = backend.status().expect("mock status");
        status.connected = false;
        backend.set_status(status);
        app.refresh(&mut backend);
        app.screen = Screen::Noise;
        app.selected = 1;
        app.handle_key(Key::Right, &mut backend);
        assert!(backend.requests.is_empty());
        assert!(app
            .message
            .as_deref()
            .is_some_and(|message| message.contains("blocked")));
    }

    #[test]
    fn notification_event_is_added_to_journal() {
        let (mut app, _) = app_and_backend();
        app.apply_event(Event::Notification {
            command: 0x0683,
            payload: vec![100],
        });
        assert_eq!(app.notifications.len(), 1);
        assert_eq!(app.notifications[0], "0x0683 64");
    }

    #[test]
    fn q_requests_exit() {
        let (mut app, mut backend) = app_and_backend();
        app.handle_key(Key::Char('q'), &mut backend);
        assert!(app.should_quit);
    }

    #[test]
    fn hfp_hint_dispatches_a2dp_profile() {
        let (mut app, mut backend) = app_and_backend();
        let mut status = backend.status().expect("mock status");
        status.audio = Some(ipc::AudioState {
            is_headset: true,
            ..Default::default()
        });
        backend.set_status(status);
        app.refresh(&mut backend);
        app.handle_key(Key::Char('a'), &mut backend);
        assert_eq!(
            backend.requests,
            vec![Request::Audio {
                set_profile: Some("a2dp-sink".to_owned())
            }]
        );
    }
}
