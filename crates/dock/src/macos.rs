use std::cell::{Cell, OnceCell, RefCell};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use ipc::{AncMode, Event, Request, Response, Status};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType, NSBox,
    NSBoxType, NSButton, NSButtonType, NSCellImagePosition, NSColor, NSControlStateValueOff,
    NSControlStateValueOn, NSFont, NSImage, NSLineBreakMode, NSMenu, NSMenuItem, NSPopUpButton,
    NSPopover, NSPopoverBehavior, NSPopoverDelegate, NSSegmentedControl, NSSlider, NSStatusBar,
    NSStatusItem, NSTextAlignment, NSTextField, NSTitlePosition, NSVariableStatusItemLength,
    NSView, NSViewController, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSArray, NSNotification, NSObject, NSPoint, NSRect, NSRectEdge,
    NSSize, NSString, NSTimer,
};
use objc2_service_management::{SMAppService, SMAppServiceStatus};

use crate::{
    anc_selection, audio_summary, charging_text, menu_bar_title, menu_tooltip, preset_selection,
};

const WINDOW_WIDTH: f64 = 760.0;
const WINDOW_HEIGHT: f64 = 660.0;
const POPOVER_WIDTH: f64 = 360.0;
const POPOVER_HEIGHT: f64 = 510.0;

#[derive(Debug)]
enum UiEvent {
    State(Box<Status>),
    Error(String),
    Offline(String),
}

#[derive(Debug)]
struct DelegateIvars {
    window: OnceCell<Retained<NSWindow>>,
    title: OnceCell<Retained<NSTextField>>,
    status: OnceCell<Retained<NSTextField>>,
    battery: OnceCell<Retained<NSTextField>>,
    battery_state: OnceCell<Retained<NSTextField>>,
    battery_bar: OnceCell<Retained<NSBox>>,
    audio: OnceCell<Retained<NSTextField>>,
    playing: OnceCell<Retained<NSTextField>>,
    anc: OnceCell<Retained<NSSegmentedControl>>,
    transparency: OnceCell<Retained<NSSlider>>,
    transparency_label: OnceCell<Retained<NSTextField>>,
    preset: OnceCell<Retained<NSPopUpButton>>,
    bass: OnceCell<Retained<NSButton>>,
    touch: OnceCell<Retained<NSButton>>,
    window_connection: OnceCell<Retained<NSButton>>,
    eq_sliders: [OnceCell<Retained<NSSlider>>; 5],
    eq_values: [OnceCell<Retained<NSTextField>>; 5],
    paired_summary: OnceCell<Retained<NSTextField>>,
    footer_detail: OnceCell<Retained<NSTextField>>,
    peer: OnceCell<Retained<NSPopUpButton>>,
    status_item: OnceCell<Retained<NSStatusItem>>,
    popover: OnceCell<Retained<NSPopover>>,
    popover_controller: OnceCell<Retained<NSViewController>>,
    popover_closed_at: Cell<Option<Instant>>,
    popover_programmatic_close: Cell<bool>,
    pop_battery: OnceCell<Retained<NSTextField>>,
    pop_battery_state: OnceCell<Retained<NSTextField>>,
    pop_status: OnceCell<Retained<NSTextField>>,
    pop_playing: OnceCell<Retained<NSTextField>>,
    pop_audio: OnceCell<Retained<NSTextField>>,
    pop_anc: OnceCell<Retained<NSSegmentedControl>>,
    pop_transparency: OnceCell<Retained<NSSlider>>,
    pop_transparency_label: OnceCell<Retained<NSTextField>>,
    pop_preset: OnceCell<Retained<NSPopUpButton>>,
    pop_bass: OnceCell<Retained<NSButton>>,
    pop_touch: OnceCell<Retained<NSButton>>,
    pop_connection: OnceCell<Retained<NSButton>>,
    dock_item: OnceCell<Retained<NSMenuItem>>,
    last_status: RefCell<Option<Status>>,
    events: Receiver<UiEvent>,
    sender: Sender<UiEvent>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            let mtm = self.mtm();
            let app = NSApplication::sharedApplication(mtm);
            let menu_bar_only = std::env::var_os("M4_MENU_BAR_ONLY").is_some();
            app.setActivationPolicy(if menu_bar_only {
                NSApplicationActivationPolicy::Accessory
            } else {
                NSApplicationActivationPolicy::Regular
            });

            if std::env::var_os("M4_NO_AUTOSTART").is_none()
                && std::env::current_exe()
                    .is_ok_and(|path| path.to_string_lossy().contains(".app/"))
            {
                // SAFETY: the main app service is queried and registered on the main thread.
                let service = unsafe { SMAppService::mainAppService() };
                if unsafe { service.status() } == SMAppServiceStatus::NotRegistered {
                    if let Err(error) = unsafe { service.registerAndReturnError() } {
                        eprintln!("m4-dock: cannot enable launch at login: {error}");
                    }
                }
            }

            let app_menu = NSMenu::new(mtm);
            app_menu.setTitle(ns_string!("M4"));
            let open = unsafe {
                app_menu.addItemWithTitle_action_keyEquivalent(
                    ns_string!("Open M4"),
                    Some(sel!(openWindow:)),
                    ns_string!("o"),
                )
            };
            unsafe { open.setTarget(Some(self.target())) };
            app_menu.addItem(&NSMenuItem::separatorItem(mtm));
            let dock_item = unsafe {
                app_menu.addItemWithTitle_action_keyEquivalent(
                    ns_string!("Show in Dock"),
                    Some(sel!(toggleDockIcon:)),
                    ns_string!(""),
                )
            };
            dock_item.setState(if menu_bar_only {
                NSControlStateValueOff
            } else {
                NSControlStateValueOn
            });
            unsafe { dock_item.setTarget(Some(self.target())) };
            app_menu.addItem(&NSMenuItem::separatorItem(mtm));
            let quit = unsafe {
                app_menu.addItemWithTitle_action_keyEquivalent(
                    ns_string!("Quit M4"),
                    Some(sel!(quitApp:)),
                    ns_string!("q"),
                )
            };
            unsafe { quit.setTarget(Some(self.target())) };
            let main_menu = NSMenu::new(mtm);
            let app_item = NSMenuItem::new(mtm);
            app_item.setSubmenu(Some(&app_menu));
            main_menu.addItem(&app_item);
            app.setMainMenu(Some(&main_menu));

            let window = self.install_window(mtm);
            self.install_status_item(mtm);
            self.install_popover(mtm);
            self.update_controls(false);
            unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    0.1,
                    self.target(),
                    sel!(poll:),
                    None,
                    true,
                )
            };
            window.center();
            self.ivars().window.set(window).unwrap();
            self.ivars().dock_item.set(dock_item).unwrap();
            if std::env::var_os("M4_SHOW_WINDOW").is_some() {
                self.show_window();
            }
        }

        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, _app: &NSApplication, _has_visible_windows: bool) -> bool {
            self.show_window();
            true
        }
    }

    unsafe impl NSPopoverDelegate for Delegate {
        #[unsafe(method(popoverDidClose:))]
        fn popover_did_close(&self, _notification: &NSNotification) {
            if !self.ivars().popover_programmatic_close.get() {
                self.ivars().popover_closed_at.set(Some(Instant::now()));
            }
        }
    }

    unsafe impl NSWindowDelegate for Delegate {
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _sender: &NSWindow) -> bool {
            if let Some(window) = self.ivars().window.get() {
                window.orderOut(None);
            }
            false
        }
    }

    impl Delegate {
        #[unsafe(method(poll:))]
        fn poll(&self, _timer: &NSTimer) {
            while let Ok(event) = self.ivars().events.try_recv() {
                match event {
                    UiEvent::State(status) => self.update(&status),
                    UiEvent::Error(message) => {
                        let status = self.ivars().status.get().unwrap();
                        status.setStringValue(&NSString::from_str(&message));
                        status.setTextColor(Some(&NSColor::systemRedColor()));
                    }
                    UiEvent::Offline(message) => self.show_offline(&message),
                }
            }
        }

        #[unsafe(method(setAnc:))]
        fn set_anc(&self, sender: &NSSegmentedControl) {
            let mode = match sender.selectedSegment() {
                0 => AncMode::Adaptive,
                1 => AncMode::Custom,
                _ => AncMode::Off,
            };
            self.request(Request::Anc { mode });
        }

        #[unsafe(method(setTransparency:))]
        fn set_transparency(&self, sender: &NSSlider) {
            let level = sender.doubleValue().round().clamp(0.0, 100.0) as u8;
            self.set_transparency_labels(level);
            self.request(Request::Transparency { level });
        }

        #[unsafe(method(setPreset:))]
        fn set_preset(&self, sender: &NSPopUpButton) {
            let index = sender.indexOfSelectedItem();
            let name = if index == 0 {
                return;
            } else if index as usize == m4::PRESETS.len() + 1 {
                "neutral"
            } else {
                m4::PRESETS[index as usize - 1].name
            };
            self.request(Request::Preset { name: name.into() });
        }

        #[unsafe(method(setEqGain:))]
        fn set_eq_gain(&self, sender: &NSSlider) {
            let band = sender.tag().clamp(0, 4) as u8;
            let gain = sender.doubleValue().clamp(-6.0, 6.0);
            if let Some(label) = self.ivars().eq_values[usize::from(band)].get() {
                label.setStringValue(&NSString::from_str(&format!("{gain:+.1} dB")));
            }
            self.request(Request::EqGain {
                band,
                centibel: (gain * 10.0).round() as i16,
            });
        }

        #[unsafe(method(connectPeer:))]
        fn connect_peer(&self, _sender: &AnyObject) {
            if let Some(index) = self.selected_peer() {
                self.request(Request::ConnectPeer { index });
            }
        }

        #[unsafe(method(disconnectPeer:))]
        fn disconnect_peer(&self, _sender: &AnyObject) {
            if let Some(index) = self.selected_peer() {
                self.request(Request::DisconnectPeer { index });
            }
        }

        #[unsafe(method(setBassBoost:))]
        fn set_bass(&self, sender: &NSButton) {
            self.request(Request::BassBoost {
                enabled: sender.state() == NSControlStateValueOn,
            });
        }

        #[unsafe(method(setTouchLock:))]
        fn set_touch(&self, sender: &NSButton) {
            self.request(Request::TouchLock {
                enabled: sender.state() == NSControlStateValueOn,
            });
        }

        #[unsafe(method(toggleHeadsetConnection:))]
        fn toggle_headset_connection(&self, _sender: &AnyObject) {
            let connect = self
                .ivars()
                .last_status
                .borrow()
                .as_ref()
                .is_none_or(|status| !status.connected);
            self.request(Request::HeadsetConnect { connect });
        }

        #[unsafe(method(toggleDockIcon:))]
        fn toggle_dock_icon(&self, sender: &NSMenuItem) {
            let app = NSApplication::sharedApplication(self.mtm());
            let hide = app.activationPolicy() == NSApplicationActivationPolicy::Regular;
            app.setActivationPolicy(if hide {
                NSApplicationActivationPolicy::Accessory
            } else {
                NSApplicationActivationPolicy::Regular
            });
            sender.setState(if hide {
                NSControlStateValueOff
            } else {
                NSControlStateValueOn
            });
        }

        #[unsafe(method(openWindow:))]
        fn open_window(&self, _sender: &AnyObject) {
            self.show_window();
        }

        #[unsafe(method(togglePopover:))]
        fn toggle_popover(&self, _sender: &AnyObject) {
            self.toggle_popover_now();
        }

        #[unsafe(method(quitApp:))]
        fn quit_app(&self, _sender: &AnyObject) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

impl Delegate {
    fn target(&self) -> &AnyObject {
        ProtocolObject::<dyn NSObjectProtocol>::from_ref(self).as_ref()
    }

    fn request(&self, request: Request) {
        let sender = self.ivars().sender.clone();
        std::thread::spawn(move || match ipc::request(&ipc::socket_path(), &request) {
            Ok(Response::Error { message }) => {
                let _ = sender.send(UiEvent::Error(message));
            }
            Ok(_) => {}
            Err(error) => {
                let _ = sender.send(UiEvent::Error(error.to_string()));
            }
        });
    }

    fn set_transparency_labels(&self, level: u8) {
        let text = format!("Transparency {level}%");
        if let Some(label) = self.ivars().transparency_label.get() {
            label.setStringValue(&NSString::from_str(&text));
        }
        if let Some(label) = self.ivars().pop_transparency_label.get() {
            label.setStringValue(&NSString::from_str(&text));
        }
    }

    fn selected_peer(&self) -> Option<u8> {
        self.ivars()
            .peer
            .get()
            .and_then(|popup| popup.selectedItem())
            .map(|item| item.tag() as u8)
    }

    fn install_window(&self, mtm: MainThreadMarker) -> Retained<NSWindow> {
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
                ),
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(ns_string!("M4"));
        window.setDelegate(Some(ProtocolObject::from_ref(self)));
        let content = window.contentView().expect("window content view");

        let title = label_in(
            &content,
            "MOMENTUM 4",
            24.0,
            18.0,
            500.0,
            32.0,
            24.0,
            true,
            mtm,
        );
        let status = label_in(
            &content,
            "Connecting to m4d…",
            24.0,
            54.0,
            500.0,
            20.0,
            13.0,
            false,
            mtm,
        );
        status.setTextColor(Some(&NSColor::secondaryLabelColor()));

        let battery = label_in(&content, "—", 594.0, 10.0, 142.0, 44.0, 34.0, true, mtm);
        battery.setAlignment(NSTextAlignment::Right);
        battery.setTextColor(Some(&NSColor::systemGreenColor()));
        let battery_state = label_in(&content, "", 594.0, 54.0, 142.0, 18.0, 12.0, false, mtm);
        battery_state.setAlignment(NSTextAlignment::Right);
        battery_state.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let battery_bar = bar_in(&content, 594.0, 80.0, 142.0, 8.0, mtm);

        let noise_panel = panel(&content, 24.0, 112.0, 456.0, 176.0, mtm);
        panel_title(&noise_panel, "NOISE CONTROL", mtm);
        let noise = noise_panel.contentView().unwrap();
        let anc = segmented_in(
            &noise,
            16.0,
            34.0,
            424.0,
            30.0,
            &["Adaptive", "Custom", "Off"],
            self.target(),
            sel!(setAnc:),
            mtm,
        );
        let transparency_label = label_in(
            &noise,
            "Transparency 0%",
            16.0,
            76.0,
            424.0,
            20.0,
            13.0,
            false,
            mtm,
        );
        let transparency = slider_in(
            &noise,
            16.0,
            102.0,
            424.0,
            26.0,
            0.0,
            100.0,
            self.target(),
            sel!(setTransparency:),
            mtm,
        );
        let hint = label_in(
            &noise,
            "Custom keeps your level on the headset.",
            16.0,
            136.0,
            424.0,
            18.0,
            11.0,
            false,
            mtm,
        );
        hint.setTextColor(Some(&NSColor::secondaryLabelColor()));

        let sound_panel = panel(&content, 496.0, 112.0, 240.0, 176.0, mtm);
        panel_title(&sound_panel, "SOUND", mtm);
        let sound = sound_panel.contentView().unwrap();
        let preset_label = label_in(&sound, "Preset", 16.0, 32.0, 208.0, 18.0, 11.0, false, mtm);
        preset_label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let preset = preset_popup(&sound, 16.0, 52.0, 208.0, 28.0, self.target(), mtm);
        let bass = toggle_in(
            &sound,
            "Bass boost",
            16.0,
            96.0,
            208.0,
            24.0,
            self.target(),
            sel!(setBassBoost:),
            mtm,
        );
        let touch = toggle_in(
            &sound,
            "Lock touch panel",
            16.0,
            128.0,
            208.0,
            24.0,
            self.target(),
            sel!(setTouchLock:),
            mtm,
        );

        let eq_panel = panel(&content, 24.0, 304.0, 456.0, 248.0, mtm);
        panel_title(&eq_panel, "EQUALIZER", mtm);
        let eq = eq_panel.contentView().unwrap();
        for (band, frequency) in ["63", "250", "1k", "4k", "8k"].iter().enumerate() {
            let x = 20.0 + band as f64 * 84.0;
            let value = label_in(&eq, "+0.0 dB", x, 30.0, 80.0, 18.0, 11.0, true, mtm);
            value.setAlignment(NSTextAlignment::Center);
            let slider = vertical_slider_in(
                &eq,
                x + 22.0,
                56.0,
                36.0,
                140.0,
                self.target(),
                sel!(setEqGain:),
                mtm,
            );
            slider.setTag(band as isize);
            let frequency = label_in(&eq, frequency, x, 204.0, 80.0, 16.0, 11.0, false, mtm);
            frequency.setAlignment(NSTextAlignment::Center);
            frequency.setTextColor(Some(&NSColor::secondaryLabelColor()));
            let hz = label_in(&eq, "Hz", x, 222.0, 80.0, 14.0, 9.0, false, mtm);
            hz.setAlignment(NSTextAlignment::Center);
            hz.setTextColor(Some(&NSColor::tertiaryLabelColor()));
            self.ivars().eq_sliders[band].set(slider).unwrap();
            self.ivars().eq_values[band].set(value).unwrap();
        }

        let playing_panel = panel(&content, 496.0, 304.0, 240.0, 104.0, mtm);
        panel_title(&playing_panel, "NOW PLAYING", mtm);
        let playing_content = playing_panel.contentView().unwrap();
        let playing = label_in(
            &playing_content,
            "Nothing playing",
            16.0,
            34.0,
            208.0,
            38.0,
            13.0,
            true,
            mtm,
        );
        playing.setMaximumNumberOfLines(2);
        playing.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        let audio = label_in(
            &playing_content,
            "No audio stream",
            16.0,
            76.0,
            208.0,
            18.0,
            11.0,
            false,
            mtm,
        );
        audio.setTextColor(Some(&NSColor::secondaryLabelColor()));

        let devices_panel = panel(&content, 496.0, 420.0, 240.0, 176.0, mtm);
        panel_title(&devices_panel, "DEVICES", mtm);
        let devices = devices_panel.contentView().unwrap();
        let paired_summary = label_in(
            &devices,
            "No paired devices",
            16.0,
            34.0,
            208.0,
            18.0,
            11.0,
            false,
            mtm,
        );
        paired_summary.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let peer = popup_base(&devices, 16.0, 56.0, 208.0, 26.0, mtm);
        let _connect = button_in(
            &devices,
            "Connect",
            16.0,
            90.0,
            100.0,
            24.0,
            self.target(),
            sel!(connectPeer:),
            mtm,
        );
        let _disconnect = button_in(
            &devices,
            "Disconnect",
            124.0,
            90.0,
            100.0,
            24.0,
            self.target(),
            sel!(disconnectPeer:),
            mtm,
        );
        let window_connection = button_in(
            &devices,
            "Connect headset",
            16.0,
            122.0,
            208.0,
            24.0,
            self.target(),
            sel!(toggleHeadsetConnection:),
            mtm,
        );

        let footer_detail = label_in(
            &content,
            "Firmware ?",
            24.0,
            612.0,
            712.0,
            34.0,
            11.0,
            false,
            mtm,
        );
        footer_detail.setTextColor(Some(&NSColor::secondaryLabelColor()));
        footer_detail.setMaximumNumberOfLines(2);
        footer_detail.setLineBreakMode(NSLineBreakMode::ByWordWrapping);

        self.ivars().title.set(title).unwrap();
        self.ivars().status.set(status).unwrap();
        self.ivars().battery.set(battery).unwrap();
        self.ivars().battery_state.set(battery_state).unwrap();
        self.ivars().battery_bar.set(battery_bar).unwrap();
        self.ivars().audio.set(audio).unwrap();
        self.ivars().playing.set(playing).unwrap();
        self.ivars().anc.set(anc).unwrap();
        self.ivars()
            .transparency_label
            .set(transparency_label)
            .unwrap();
        self.ivars().transparency.set(transparency).unwrap();
        self.ivars().preset.set(preset).unwrap();
        self.ivars().bass.set(bass).unwrap();
        self.ivars().touch.set(touch).unwrap();
        self.ivars()
            .window_connection
            .set(window_connection)
            .unwrap();
        self.ivars().paired_summary.set(paired_summary).unwrap();
        self.ivars().peer.set(peer).unwrap();
        self.ivars().footer_detail.set(footer_detail).unwrap();
        window
    }

    fn install_status_item(&self, mtm: MainThreadMarker) {
        let status_item =
            NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        if let Some(button) = status_item.button(mtm) {
            button.setTitle(ns_string!("M4"));
            if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                ns_string!("headphones"),
                Some(ns_string!("MOMENTUM 4")),
            ) {
                image.setTemplate(true);
                button.setImage(Some(&image));
                button.setImagePosition(NSCellImagePosition::ImageLeft);
            }
            unsafe {
                button.setTarget(Some(self.target()));
                button.setAction(Some(sel!(togglePopover:)));
            }
            button.setToolTip(Some(ns_string!("MOMENTUM 4: connecting…")));
        }
        status_item.setVisible(true);
        self.ivars().status_item.set(status_item).unwrap();
    }

    fn install_popover(&self, mtm: MainThreadMarker) {
        let content = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(POPOVER_WIDTH, POPOVER_HEIGHT),
            ),
        );

        let title = label_in(
            &content,
            "MOMENTUM 4",
            20.0,
            16.0,
            220.0,
            24.0,
            18.0,
            true,
            mtm,
        );
        title.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        let status = label_in(
            &content,
            "Connecting to m4d…",
            20.0,
            42.0,
            220.0,
            16.0,
            11.0,
            false,
            mtm,
        );
        status.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let pop_battery = label_in(&content, "—", 250.0, 10.0, 90.0, 32.0, 28.0, true, mtm);
        pop_battery.setAlignment(NSTextAlignment::Right);
        pop_battery.setTextColor(Some(&NSColor::labelColor()));
        let pop_battery_state = label_in(&content, "", 250.0, 42.0, 90.0, 16.0, 10.0, false, mtm);
        pop_battery_state.setAlignment(NSTextAlignment::Right);
        pop_battery_state.setTextColor(Some(&NSColor::secondaryLabelColor()));

        let noise_panel = panel(&content, 20.0, 80.0, 320.0, 144.0, mtm);
        panel_title(&noise_panel, "NOISE CONTROL", mtm);
        let noise = noise_panel.contentView().unwrap();
        let pop_anc = segmented_in(
            &noise,
            16.0,
            30.0,
            288.0,
            30.0,
            &["Adaptive", "Custom", "Off"],
            self.target(),
            sel!(setAnc:),
            mtm,
        );
        let pop_transparency_label = label_in(
            &noise,
            "Transparency 0%",
            16.0,
            72.0,
            288.0,
            18.0,
            11.0,
            false,
            mtm,
        );
        let pop_transparency = slider_in(
            &noise,
            16.0,
            96.0,
            288.0,
            24.0,
            0.0,
            100.0,
            self.target(),
            sel!(setTransparency:),
            mtm,
        );
        let hint = label_in(
            &noise,
            "Custom keeps your level.",
            16.0,
            122.0,
            288.0,
            14.0,
            10.0,
            false,
            mtm,
        );
        hint.setTextColor(Some(&NSColor::tertiaryLabelColor()));

        let sound_panel = panel(&content, 20.0, 236.0, 320.0, 122.0, mtm);
        panel_title(&sound_panel, "SOUND", mtm);
        let sound = sound_panel.contentView().unwrap();
        let pop_preset = preset_popup(&sound, 16.0, 32.0, 150.0, 28.0, self.target(), mtm);
        let pop_bass = toggle_in(
            &sound,
            "Bass boost",
            180.0,
            34.0,
            124.0,
            24.0,
            self.target(),
            sel!(setBassBoost:),
            mtm,
        );
        let pop_touch = toggle_in(
            &sound,
            "Lock touch panel",
            16.0,
            76.0,
            288.0,
            24.0,
            self.target(),
            sel!(setTouchLock:),
            mtm,
        );

        let playing_panel = panel(&content, 20.0, 370.0, 320.0, 84.0, mtm);
        panel_title(&playing_panel, "NOW PLAYING", mtm);
        let playing_content = playing_panel.contentView().unwrap();
        let pop_playing = label_in(
            &playing_content,
            "Nothing playing",
            16.0,
            30.0,
            288.0,
            30.0,
            12.0,
            true,
            mtm,
        );
        pop_playing.setMaximumNumberOfLines(2);
        pop_playing.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        let pop_audio = label_in(
            &playing_content,
            "No audio stream",
            16.0,
            62.0,
            288.0,
            14.0,
            10.0,
            false,
            mtm,
        );
        pop_audio.setTextColor(Some(&NSColor::secondaryLabelColor()));

        let pop_connection = button_in(
            &content,
            "Connect",
            20.0,
            466.0,
            110.0,
            30.0,
            self.target(),
            sel!(toggleHeadsetConnection:),
            mtm,
        );
        let _open = button_in(
            &content,
            "Open M4…",
            138.0,
            466.0,
            140.0,
            30.0,
            self.target(),
            sel!(openWindow:),
            mtm,
        );
        let _quit = button_in(
            &content,
            "Quit",
            286.0,
            466.0,
            54.0,
            30.0,
            self.target(),
            sel!(quitApp:),
            mtm,
        );
        let controller = NSViewController::new(mtm);
        controller.setView(&content);
        let popover = NSPopover::new(mtm);
        popover.setContentViewController(Some(&controller));
        popover.setContentSize(NSSize::new(POPOVER_WIDTH, POPOVER_HEIGHT));
        popover.setBehavior(NSPopoverBehavior::Transient);
        popover.setAnimates(true);
        popover.setDelegate(Some(ProtocolObject::from_ref(self)));

        self.ivars().pop_battery.set(pop_battery).unwrap();
        self.ivars()
            .pop_battery_state
            .set(pop_battery_state)
            .unwrap();
        self.ivars().pop_status.set(status).unwrap();
        self.ivars().pop_playing.set(pop_playing).unwrap();
        self.ivars().pop_audio.set(pop_audio).unwrap();
        self.ivars().pop_anc.set(pop_anc).unwrap();
        self.ivars()
            .pop_transparency_label
            .set(pop_transparency_label)
            .unwrap();
        self.ivars().pop_transparency.set(pop_transparency).unwrap();
        self.ivars().pop_preset.set(pop_preset).unwrap();
        self.ivars().pop_bass.set(pop_bass).unwrap();
        self.ivars().pop_touch.set(pop_touch).unwrap();
        self.ivars().pop_connection.set(pop_connection).unwrap();
        self.ivars().popover.set(popover).unwrap();
        self.ivars().popover_controller.set(controller).unwrap();
    }

    fn update(&self, status: &Status) {
        self.ivars().last_status.replace(Some(status.clone()));
        self.update_status_item(status);
        self.update_controls(status.connected);

        let status_label = self.ivars().status.get().unwrap();
        let status_color = if status.connected {
            NSColor::systemGreenColor()
        } else {
            NSColor::systemRedColor()
        };
        status_label.setTextColor(Some(&status_color));

        self.update_window(status);
        self.update_popover(status);
    }

    fn update_window(&self, status: &Status) {
        let status_label = self.ivars().status.get().unwrap();
        if !status.connected {
            status_label.setStringValue(&NSString::from_str(
                status.error.as_deref().unwrap_or("Headset disconnected"),
            ));
            self.ivars()
                .battery
                .get()
                .unwrap()
                .setStringValue(ns_string!("—"));
            self.ivars()
                .battery_state
                .get()
                .unwrap()
                .setStringValue(ns_string!(""));
            let bar = self.ivars().battery_bar.get().unwrap();
            let mut frame = bar.frame();
            frame.size.width = 0.0;
            bar.setFrame(frame);
            self.ivars()
                .audio
                .get()
                .unwrap()
                .setStringValue(ns_string!(""));
            self.ivars()
                .playing
                .get()
                .unwrap()
                .setStringValue(ns_string!("Headset disconnected"));
            self.ivars()
                .window_connection
                .get()
                .unwrap()
                .setTitle(ns_string!("Connect headset"));
            self.ivars()
                .paired_summary
                .get()
                .unwrap()
                .setStringValue(ns_string!("Headset disconnected"));
            if let Some(peer) = self.ivars().peer.get() {
                peer.removeAllItems();
            }
            self.ivars()
                .footer_detail
                .get()
                .unwrap()
                .setStringValue(ns_string!("m4d is waiting for the headset"));
            return;
        }

        status_label.setStringValue(ns_string!("Connected"));
        self.ivars()
            .title
            .get()
            .unwrap()
            .setStringValue(ns_string!("MOMENTUM 4"));
        self.ivars()
            .battery
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(&match status.info.battery_percent {
                Some(percent) => format!("{percent}%"),
                None => "—".into(),
            }));
        self.ivars()
            .battery_state
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(&charging_text(
                status.info.charging.as_deref(),
            )));
        let percent = status.info.battery_percent.unwrap_or(0);
        let battery_color = if percent <= 10 {
            NSColor::systemRedColor()
        } else if percent <= 20 {
            NSColor::systemOrangeColor()
        } else {
            NSColor::systemGreenColor()
        };
        let bar = self.ivars().battery_bar.get().unwrap();
        let mut frame = bar.frame();
        frame.size.width = 142.0 * f64::from(percent) / 100.0;
        bar.setFrame(frame);
        bar.setFillColor(&battery_color);
        self.ivars()
            .audio
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(&audio_summary(&status.info)));
        self.ivars()
            .playing
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(
                &status
                    .now_playing
                    .as_ref()
                    .map(|track| format!("Now playing\n{}", track.label()))
                    .unwrap_or_else(|| "Nothing playing".to_owned()),
            ));
        self.ivars()
            .anc
            .get()
            .unwrap()
            .setSelectedSegment(anc_selection(&status.info) as isize);
        let level = status.info.transparency.unwrap_or(0);
        self.ivars()
            .transparency
            .get()
            .unwrap()
            .setDoubleValue(level.into());
        self.set_transparency_labels(level);
        let preset = preset_index(&status.info);
        self.ivars()
            .preset
            .get()
            .unwrap()
            .selectItemAtIndex(preset as isize);
        self.ivars()
            .bass
            .get()
            .unwrap()
            .setState(if status.info.bass_boost == Some(true) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        self.ivars()
            .touch
            .get()
            .unwrap()
            .setState(if status.info.touch_locked == Some(true) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        for band in 0..5 {
            let gain = status.info.eq_gains_db.get(band).copied().unwrap_or(0.0);
            self.ivars().eq_sliders[band]
                .get()
                .unwrap()
                .setDoubleValue(gain.into());
            self.ivars().eq_values[band]
                .get()
                .unwrap()
                .setStringValue(&NSString::from_str(&format!("{gain:+.1} dB")));
        }
        self.ivars()
            .window_connection
            .get()
            .unwrap()
            .setTitle(ns_string!("Disconnect headset"));
        self.ivars()
            .paired_summary
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(&paired_summary_text(status)));
        if let Some(peer) = self.ivars().peer.get() {
            // Preserve the selected device across refreshes.
            let previous = peer.selectedItem().map(|item| item.tag());
            peer.removeAllItems();
            for device in &status.info.paired {
                peer.addItemWithTitle(&NSString::from_str(&format!(
                    "{} [{:02}] {}{}",
                    if device.connected { "●" } else { "○" },
                    device.index,
                    device.name,
                    if Some(device.index) == status.info.own_index {
                        " · host"
                    } else {
                        ""
                    }
                )));
                if let Some(item) = peer.itemAtIndex(peer.numberOfItems() - 1) {
                    item.setTag(device.index.into());
                }
            }
            let preferred = previous
                .filter(|tag| {
                    status
                        .info
                        .paired
                        .iter()
                        .any(|device| isize::from(device.index) == *tag)
                })
                .or_else(|| {
                    status
                        .info
                        .own_index
                        .and_then(|index| {
                            status
                                .info
                                .paired
                                .iter()
                                .position(|device| device.index == index)
                        })
                        .map(|row| status.info.paired[row].index.into())
                })
                .or_else(|| {
                    status
                        .info
                        .paired
                        .iter()
                        .find(|device| device.connected)
                        .map(|device| device.index.into())
                });
            if let Some(tag) = preferred {
                if let Some(row) = status
                    .info
                    .paired
                    .iter()
                    .position(|device| isize::from(device.index) == tag)
                {
                    peer.selectItemAtIndex(row as isize);
                }
            }
        }
        self.ivars()
            .footer_detail
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(&format!(
            "Firmware {} · GAIA {} · Hardware {}\nSerial {} · Audio mode {} · Bluetooth mode {}",
            status.info.firmware.as_deref().unwrap_or("?"),
            status.info.gaia_api.as_deref().unwrap_or("?"),
            status
                .info
                .hardware_revision
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".into()),
            status.info.serial.as_deref().unwrap_or("?"),
            status.info.audio_mode.as_deref().unwrap_or("?"),
            status.info.bluetooth_mode.as_deref().unwrap_or("?"),
        )));
    }

    fn update_popover(&self, status: &Status) {
        self.ivars()
            .pop_status
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(if status.connected {
                "Connected"
            } else {
                status.error.as_deref().unwrap_or("Disconnected")
            }));
        let pop_status_color = if status.connected {
            NSColor::labelColor()
        } else {
            NSColor::systemRedColor()
        };
        self.ivars()
            .pop_status
            .get()
            .unwrap()
            .setTextColor(Some(&pop_status_color));
        self.ivars()
            .pop_battery
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(&match status.info.battery_percent {
                Some(percent) if status.connected => format!("{percent}%"),
                _ => "—".into(),
            }));
        let battery_state = if status.connected {
            charging_text(status.info.charging.as_deref())
        } else {
            String::new()
        };
        self.ivars()
            .pop_battery_state
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(&battery_state));
        self.ivars()
            .pop_connection
            .get()
            .unwrap()
            .setTitle(&NSString::from_str(if status.connected {
                "Disconnect"
            } else {
                "Connect"
            }));
        if !status.connected {
            self.ivars()
                .pop_playing
                .get()
                .unwrap()
                .setStringValue(ns_string!("m4d is waiting for the headset"));
            self.ivars()
                .pop_audio
                .get()
                .unwrap()
                .setStringValue(ns_string!(""));
            return;
        }
        self.ivars()
            .pop_playing
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(
                &status
                    .now_playing
                    .as_ref()
                    .map(|track| track.label())
                    .unwrap_or_else(|| "Nothing playing".to_owned()),
            ));
        self.ivars()
            .pop_audio
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(&audio_summary(&status.info)));
        self.ivars()
            .pop_anc
            .get()
            .unwrap()
            .setSelectedSegment(anc_selection(&status.info) as isize);
        let level = status.info.transparency.unwrap_or(0);
        self.ivars()
            .pop_transparency
            .get()
            .unwrap()
            .setDoubleValue(level.into());
        self.ivars()
            .pop_transparency_label
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(&format!("Transparency {level}%")));
        self.ivars()
            .pop_preset
            .get()
            .unwrap()
            .selectItemAtIndex(preset_index(&status.info) as isize);
        self.ivars()
            .pop_bass
            .get()
            .unwrap()
            .setState(if status.info.bass_boost == Some(true) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        self.ivars()
            .pop_touch
            .get()
            .unwrap()
            .setState(if status.info.touch_locked == Some(true) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
    }

    fn update_status_item(&self, status: &Status) {
        if let Some(item) = self.ivars().status_item.get() {
            if let Some(button) = item.button(self.mtm()) {
                button.setTitle(&NSString::from_str(&menu_bar_title(status)));
                button.setToolTip(Some(&NSString::from_str(&menu_tooltip(status))));
            }
        }
    }

    fn update_controls(&self, enabled: bool) {
        if let Some(control) = self.ivars().anc.get() {
            control.setEnabled(enabled);
        }
        if let Some(control) = self.ivars().transparency.get() {
            control.setEnabled(enabled);
        }
        if let Some(control) = self.ivars().preset.get() {
            control.setEnabled(enabled);
        }
        if let Some(control) = self.ivars().bass.get() {
            control.setEnabled(enabled);
        }
        if let Some(control) = self.ivars().touch.get() {
            control.setEnabled(enabled);
        }
        if let Some(control) = self.ivars().peer.get() {
            control.setEnabled(enabled);
        }
        if let Some(control) = self.ivars().pop_anc.get() {
            control.setEnabled(enabled);
        }
        if let Some(control) = self.ivars().pop_transparency.get() {
            control.setEnabled(enabled);
        }
        if let Some(control) = self.ivars().pop_preset.get() {
            control.setEnabled(enabled);
        }
        if let Some(control) = self.ivars().pop_bass.get() {
            control.setEnabled(enabled);
        }
        if let Some(control) = self.ivars().pop_touch.get() {
            control.setEnabled(enabled);
        }
        for slider in self.ivars().eq_sliders.iter().filter_map(OnceCell::get) {
            slider.setEnabled(enabled);
        }
        if let Some(button) = self.ivars().window_connection.get() {
            button.setEnabled(true);
        }
        if let Some(button) = self.ivars().pop_connection.get() {
            button.setEnabled(true);
        }
    }

    fn show_offline(&self, message: &str) {
        self.update_controls(false);
        self.ivars().last_status.replace(None);
        self.ivars()
            .title
            .get()
            .unwrap()
            .setStringValue(ns_string!("MOMENTUM 4"));
        let status_label = self.ivars().status.get().unwrap();
        status_label.setStringValue(&NSString::from_str(message));
        status_label.setTextColor(Some(&NSColor::systemRedColor()));
        self.ivars()
            .battery
            .get()
            .unwrap()
            .setStringValue(ns_string!("—"));
        self.ivars()
            .battery_state
            .get()
            .unwrap()
            .setStringValue(ns_string!(""));
        let bar = self.ivars().battery_bar.get().unwrap();
        let mut frame = bar.frame();
        frame.size.width = 0.0;
        bar.setFrame(frame);
        self.ivars()
            .audio
            .get()
            .unwrap()
            .setStringValue(ns_string!(""));
        self.ivars()
            .playing
            .get()
            .unwrap()
            .setStringValue(ns_string!("m4d is not running"));
        self.ivars()
            .paired_summary
            .get()
            .unwrap()
            .setStringValue(ns_string!("m4d is not running"));
        if let Some(peer) = self.ivars().peer.get() {
            peer.removeAllItems();
        }
        self.ivars()
            .footer_detail
            .get()
            .unwrap()
            .setStringValue(ns_string!("m4d is not running"));
        self.ivars()
            .pop_status
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(message));
        self.ivars()
            .pop_status
            .get()
            .unwrap()
            .setTextColor(Some(&NSColor::systemRedColor()));
        self.ivars()
            .pop_battery
            .get()
            .unwrap()
            .setStringValue(ns_string!("—"));
        self.ivars()
            .pop_battery_state
            .get()
            .unwrap()
            .setStringValue(ns_string!(""));
        self.ivars()
            .pop_playing
            .get()
            .unwrap()
            .setStringValue(ns_string!("m4d is not running"));
        self.ivars()
            .pop_audio
            .get()
            .unwrap()
            .setStringValue(ns_string!(""));
        self.ivars()
            .pop_connection
            .get()
            .unwrap()
            .setTitle(ns_string!("Connect"));
        self.ivars()
            .window_connection
            .get()
            .unwrap()
            .setTitle(ns_string!("Connect headset"));
        if let Some(item) = self.ivars().status_item.get() {
            if let Some(button) = item.button(self.mtm()) {
                button.setTitle(ns_string!("M4 off"));
                button.setToolTip(Some(&NSString::from_str(message)));
            }
        }
    }

    fn close_popover_programmatically(&self) {
        if let Some(popover) = self.ivars().popover.get() {
            if popover.isShown() {
                self.ivars().popover_programmatic_close.set(true);
                popover.close();
            }
        }
    }

    fn show_window(&self) {
        self.close_popover_programmatically();
        if let Some(window) = self.ivars().window.get() {
            let app = NSApplication::sharedApplication(self.mtm());
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            if let Some(item) = self.ivars().dock_item.get() {
                item.setState(NSControlStateValueOn);
            }
            window.makeKeyAndOrderFront(None);
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
        }
    }

    fn toggle_popover_now(&self) {
        let Some(popover) = self.ivars().popover.get() else {
            return;
        };
        if popover.isShown() {
            self.close_popover_programmatically();
            return;
        }
        // Transient close arrives before the status-item click.
        if let Some(closed) = self.ivars().popover_closed_at.get() {
            if closed.elapsed() < Duration::from_millis(300) {
                self.ivars().popover_closed_at.set(None);
                return;
            }
        }
        let Some(item) = self.ivars().status_item.get() else {
            return;
        };
        let Some(button) = item.button(self.mtm()) else {
            return;
        };
        self.ivars().popover_programmatic_close.set(false);
        self.ivars().popover_closed_at.set(None);
        #[allow(deprecated)]
        NSApplication::sharedApplication(self.mtm()).activateIgnoringOtherApps(true);
        popover.showRelativeToRect_ofView_preferredEdge(button.bounds(), &button, NSRectEdge::MaxY);
    }

    fn new(mtm: MainThreadMarker) -> (Retained<Self>, Sender<UiEvent>) {
        let (sender, events) = mpsc::channel();
        let ivars = DelegateIvars {
            window: OnceCell::new(),
            title: OnceCell::new(),
            status: OnceCell::new(),
            battery: OnceCell::new(),
            battery_state: OnceCell::new(),
            battery_bar: OnceCell::new(),
            audio: OnceCell::new(),
            playing: OnceCell::new(),
            anc: OnceCell::new(),
            transparency: OnceCell::new(),
            transparency_label: OnceCell::new(),
            preset: OnceCell::new(),
            bass: OnceCell::new(),
            touch: OnceCell::new(),
            window_connection: OnceCell::new(),
            eq_sliders: std::array::from_fn(|_| OnceCell::new()),
            eq_values: std::array::from_fn(|_| OnceCell::new()),
            paired_summary: OnceCell::new(),
            footer_detail: OnceCell::new(),
            peer: OnceCell::new(),
            status_item: OnceCell::new(),
            popover: OnceCell::new(),
            popover_controller: OnceCell::new(),
            popover_closed_at: Cell::new(None),
            popover_programmatic_close: Cell::new(false),
            pop_battery: OnceCell::new(),
            pop_battery_state: OnceCell::new(),
            pop_status: OnceCell::new(),
            pop_playing: OnceCell::new(),
            pop_audio: OnceCell::new(),
            pop_anc: OnceCell::new(),
            pop_transparency: OnceCell::new(),
            pop_transparency_label: OnceCell::new(),
            pop_preset: OnceCell::new(),
            pop_bass: OnceCell::new(),
            pop_touch: OnceCell::new(),
            pop_connection: OnceCell::new(),
            dock_item: OnceCell::new(),
            last_status: RefCell::new(None),
            events,
            sender: sender.clone(),
        };
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: the superclass is NSObject and its init signature is unchanged.
        let delegate = unsafe { msg_send![super(this), init] };
        (delegate, sender)
    }
}

fn top_rect(container: &NSView, x: f64, top: f64, width: f64, height: f64) -> NSRect {
    let container_height = container.bounds().size.height;
    NSRect::new(
        NSPoint::new(x, container_height - top - height),
        NSSize::new(width, height),
    )
}

fn font(size: f64, bold: bool) -> Retained<NSFont> {
    if bold {
        NSFont::boldSystemFontOfSize(size)
    } else {
        NSFont::systemFontOfSize(size)
    }
}

#[allow(clippy::too_many_arguments)]
fn label_in(
    container: &NSView,
    text: &str,
    x: f64,
    top: f64,
    width: f64,
    height: f64,
    size: f64,
    bold: bool,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(top_rect(container, x, top, width, height));
    label.setFont(Some(&font(size, bold)));
    container.addSubview(&label);
    label
}

fn panel(
    parent: &NSView,
    x: f64,
    top: f64,
    width: f64,
    height: f64,
    mtm: MainThreadMarker,
) -> Retained<NSBox> {
    let panel = NSBox::initWithFrame(NSBox::alloc(mtm), top_rect(parent, x, top, width, height));
    panel.setBoxType(NSBoxType::Custom);
    panel.setTitlePosition(NSTitlePosition::NoTitle);
    panel.setBorderWidth(1.0);
    panel.setCornerRadius(12.0);
    panel.setFillColor(&NSColor::controlBackgroundColor());
    panel.setBorderColor(&NSColor::separatorColor());
    parent.addSubview(&panel);
    panel
}

fn panel_title(panel: &NSBox, text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let content = panel.contentView().unwrap();
    let label = label_in(
        &content,
        text,
        16.0,
        12.0,
        content.bounds().size.width - 32.0,
        16.0,
        11.0,
        true,
        mtm,
    );
    label.setTextColor(Some(&NSColor::secondaryLabelColor()));
    label
}

fn bar_in(
    container: &NSView,
    x: f64,
    top: f64,
    width: f64,
    height: f64,
    mtm: MainThreadMarker,
) -> Retained<NSBox> {
    let background = NSBox::initWithFrame(
        NSBox::alloc(mtm),
        top_rect(container, x, top, width, height),
    );
    background.setBoxType(NSBoxType::Custom);
    background.setTitlePosition(NSTitlePosition::NoTitle);
    background.setBorderWidth(0.0);
    background.setCornerRadius(height / 2.0);
    background.setFillColor(&NSColor::separatorColor());
    container.addSubview(&background);

    let fill = NSBox::initWithFrame(NSBox::alloc(mtm), top_rect(container, x, top, 0.0, height));
    fill.setBoxType(NSBoxType::Custom);
    fill.setTitlePosition(NSTitlePosition::NoTitle);
    fill.setBorderWidth(0.0);
    fill.setCornerRadius(height / 2.0);
    fill.setFillColor(&NSColor::systemGreenColor());
    container.addSubview(&fill);
    fill
}

#[allow(clippy::too_many_arguments)]
fn segmented_in(
    container: &NSView,
    x: f64,
    top: f64,
    width: f64,
    height: f64,
    labels: &[&str],
    target: &AnyObject,
    action: Sel,
    mtm: MainThreadMarker,
) -> Retained<NSSegmentedControl> {
    let control = NSSegmentedControl::initWithFrame(
        NSSegmentedControl::alloc(mtm),
        top_rect(container, x, top, width, height),
    );
    control.setSegmentCount(labels.len() as isize);
    for (index, label) in labels.iter().enumerate() {
        control.setLabel_forSegment(&NSString::from_str(label), index as isize);
    }
    unsafe {
        control.setTarget(Some(target));
        control.setAction(Some(action));
    }
    container.addSubview(&control);
    control
}

#[allow(clippy::too_many_arguments)]
fn slider_in(
    container: &NSView,
    x: f64,
    top: f64,
    width: f64,
    height: f64,
    min: f64,
    max: f64,
    target: &AnyObject,
    action: Sel,
    mtm: MainThreadMarker,
) -> Retained<NSSlider> {
    let slider = NSSlider::initWithFrame(
        NSSlider::alloc(mtm),
        top_rect(container, x, top, width, height),
    );
    slider.setMinValue(min);
    slider.setMaxValue(max);
    slider.setContinuous(false);
    unsafe {
        slider.setTarget(Some(target));
        slider.setAction(Some(action));
    }
    container.addSubview(&slider);
    slider
}

#[allow(clippy::too_many_arguments)]
fn vertical_slider_in(
    container: &NSView,
    x: f64,
    top: f64,
    width: f64,
    height: f64,
    target: &AnyObject,
    action: Sel,
    mtm: MainThreadMarker,
) -> Retained<NSSlider> {
    let slider = slider_in(
        container, x, top, width, height, -6.0, 6.0, target, action, mtm,
    );
    slider.setVertical(true);
    slider
}

#[allow(clippy::too_many_arguments)]
fn toggle_in(
    container: &NSView,
    title: &str,
    x: f64,
    top: f64,
    width: f64,
    height: f64,
    target: &AnyObject,
    action: Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(target),
            Some(action),
            mtm,
        )
    };
    button.setButtonType(NSButtonType::Switch);
    button.setFrame(top_rect(container, x, top, width, height));
    container.addSubview(&button);
    button
}

#[allow(clippy::too_many_arguments)]
fn button_in(
    container: &NSView,
    title: &str,
    x: f64,
    top: f64,
    width: f64,
    height: f64,
    target: &AnyObject,
    action: Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(target),
            Some(action),
            mtm,
        )
    };
    button.setFrame(top_rect(container, x, top, width, height));
    container.addSubview(&button);
    button
}

fn popup_base(
    container: &NSView,
    x: f64,
    top: f64,
    width: f64,
    height: f64,
    mtm: MainThreadMarker,
) -> Retained<NSPopUpButton> {
    let popup = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        top_rect(container, x, top, width, height),
        false,
    );
    container.addSubview(&popup);
    popup
}

fn preset_popup(
    container: &NSView,
    x: f64,
    top: f64,
    width: f64,
    height: f64,
    target: &AnyObject,
    mtm: MainThreadMarker,
) -> Retained<NSPopUpButton> {
    let popup = popup_base(container, x, top, width, height, mtm);
    let mut titles = vec![NSString::from_str("Custom EQ")];
    titles.extend(
        m4::PRESETS
            .iter()
            .map(|preset| NSString::from_str(preset.name)),
    );
    titles.push(NSString::from_str("neutral"));
    popup.addItemsWithTitles(&NSArray::from_retained_slice(&titles));
    unsafe {
        popup.setTarget(Some(target));
        popup.setAction(Some(sel!(setPreset:)));
    }
    popup
}

fn preset_index(info: &m4::Info) -> usize {
    match preset_selection(info) {
        Some(index) if index < m4::PRESETS.len() => index + 1,
        Some(_) => m4::PRESETS.len() + 1,
        None => 0,
    }
}

fn paired_summary_text(status: &Status) -> String {
    if status.info.paired.is_empty() {
        return "No paired devices".to_owned();
    }
    let connected = status
        .info
        .paired
        .iter()
        .filter(|device| device.connected)
        .count();
    format!(
        "{} paired · {} connected",
        status.info.paired.len(),
        connected
    )
}

fn watch(sender: Sender<UiEvent>) {
    loop {
        let result = ipc::watch(&ipc::socket_path(), |event| {
            if let Event::State { status } = event {
                let _ = sender.send(UiEvent::State(Box::new(status)));
            }
        });
        if let Err(error) = result {
            let _ = sender.send(UiEvent::Offline(format!("m4d: {error}")));
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

pub fn main() {
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    let (delegate, sender) = Delegate::new(mtm);
    std::thread::spawn(move || watch(sender));
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.finishLaunching();
    app.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paired_summary_counts_connected_hosts() {
        let mut status = Status {
            connected: true,
            ..Status::default()
        };
        assert_eq!(paired_summary_text(&status), "No paired devices");
        status.info.paired.push(m4::PairedEntry {
            index: 0,
            name: "phone".to_owned(),
            connected: false,
            priority: 1,
        });
        status.info.paired.push(m4::PairedEntry {
            index: 1,
            name: "laptop".to_owned(),
            connected: true,
            priority: 0,
        });
        assert_eq!(paired_summary_text(&status), "2 paired · 1 connected");
    }

    #[test]
    fn preset_index_matches_the_popup_order() {
        let mut info = m4::Info::default();
        assert_eq!(preset_index(&info), 0);
        info.eq_gains_db = m4::PRESETS[0].gains_db.to_vec();
        assert_eq!(preset_index(&info), 1);
        info.audio_mode = Some("off (headset tuning)".to_owned());
        assert_eq!(preset_index(&info), m4::PRESETS.len() + 1);
    }
}
