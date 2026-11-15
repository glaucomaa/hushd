use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Paragraph, Widget, Wrap};

use ipc::AncMode;

use crate::app::{App, AppLayout, DeviceAction, Screen};

const BG_HL: Color = Color::Rgb(0x29, 0x2e, 0x42);
const BORDER: Color = Color::Rgb(0x2f, 0x35, 0x49);
const FG: Color = Color::Rgb(0xa9, 0xb1, 0xd6);
const BRIGHT: Color = Color::Rgb(0xc0, 0xca, 0xf5);
const DIM: Color = Color::Rgb(0x56, 0x5f, 0x89);
const ACCENT: Color = Color::Rgb(0x7a, 0xa2, 0xf7);
const GREEN: Color = Color::Rgb(0x9e, 0xce, 0x6a);
const YELLOW: Color = Color::Rgb(0xe0, 0xaf, 0x68);
const ORANGE: Color = Color::Rgb(0xff, 0x9e, 0x64);
const RED: Color = Color::Rgb(0xf7, 0x76, 0x8e);
const CYAN: Color = Color::Rgb(0x7d, 0xcf, 0xff);

pub fn render(app: &App, frame: &mut Frame) {
    if app.show_help {
        frame.render_widget(help(), frame.area());
        return;
    }
    match app.layout {
        AppLayout::Instrument => instrument(app, frame),
        AppLayout::Minimal => minimal(app, frame),
    }
    if app.show_presets {
        preset_picker(app, frame);
    }
}

fn preset_picker(app: &App, frame: &mut Frame) {
    let names = crate::app::preset_names();
    let width = 44u16.min(frame.area().width);
    let height = (names.len() as u16 + 3).min(frame.area().height);
    let area = Rect {
        x: frame.area().x + (frame.area().width.saturating_sub(width)) / 2,
        y: frame.area().y + (frame.area().height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(ratatui::widgets::Clear, area);
    let block = Block::bordered()
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            " presets ".to_owned(),
            Style::default().fg(BRIGHT),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let selected = index == app.preset_selected;
        let active = active_preset(app).as_deref() == Some(*name);
        let gains = crate::app::preset_gains(name);
        let detail = match gains {
            Some(gains) => gains
                .iter()
                .map(|gain| format!("{gain:+.1}"))
                .collect::<Vec<_>>()
                .join(" "),
            None => "flat, headset tuning".to_owned(),
        };
        let style = if selected {
            Style::default()
                .bg(BG_HL)
                .fg(BRIGHT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(FG)
        };
        lines.push(Line::from(Span::styled(
            format!(
                " {} {:<10} {:>18}",
                if selected {
                    "▸"
                } else if active {
                    "●"
                } else {
                    " "
                },
                name,
                detail
            ),
            style,
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

// ---------------------------------------------------------------- instrument

fn instrument(app: &App, frame: &mut Frame) {
    *app.hitboxes.borrow_mut() = crate::app::Hitboxes::default();
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(8),
        Constraint::Length(3),
    ])
    .split(frame.area());
    frame.render_widget(header(app), rows[0]);
    match app.screen {
        Screen::Overview => overview_cards(app, frame, rows[1]),
        Screen::Noise => noise_cards(app, frame, rows[1]),
        Screen::Sound => sound_cards(app, frame, rows[1]),
        Screen::Devices => devices_card(app, frame, rows[1]),
        Screen::Diagnostics => diagnostics_cards(app, frame, rows[1]),
    }
    frame.render_widget(footer(app, true), rows[2]);
}

fn header(app: &App) -> Paragraph<'static> {
    let info = &app.status.info;
    let model = info.model.as_deref().unwrap_or("MOMENTUM 4");
    let (connection, connection_color) = if app.status.connected {
        let text = match app.status.channel {
            Some(channel) => format!("connected · RFCOMM {channel}"),
            None => "connected".to_owned(),
        };
        (text, GREEN)
    } else {
        ("headset not connected".to_owned(), RED)
    };
    let battery = info
        .battery_percent
        .map(|percent| format!("󰂂 {percent}%"))
        .unwrap_or_else(|| "󰂂 ?".to_owned());
    let codec = app
        .status
        .audio
        .as_ref()
        .and_then(|audio| audio.codec.clone())
        .map(|codec| codec_label(&codec))
        .or_else(|| info.codec.clone());
    let rate = app
        .status
        .audio
        .as_ref()
        .and_then(|audio| audio.sample_rate_hz)
        .or(info.sample_rate_hz);

    let status_line = Line::from(vec![
        Span::styled(
            format!(" {model} "),
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::default().fg(DIM)),
        Span::styled(connection, Style::default().fg(connection_color)),
        Span::styled("  ·  ", Style::default().fg(DIM)),
        Span::styled(battery, Style::default().fg(GREEN)),
        Span::styled("  ·  ", Style::default().fg(DIM)),
        Span::styled(
            codec.unwrap_or_else(|| "?".to_owned()),
            Style::default().fg(FG),
        ),
        Span::styled(
            rate.map(|hz| format!(" {hz} Hz")).unwrap_or_default(),
            Style::default().fg(DIM),
        ),
        Span::styled("  ·  ", Style::default().fg(DIM)),
        Span::styled(
            info.firmware.clone().unwrap_or_else(|| "?".to_owned()),
            Style::default().fg(DIM),
        ),
    ]);

    let mut tabs = vec![Span::styled(" ", Style::default())];
    for index in 1..=5u8 {
        let Some(screen) = Screen::from_number(index) else {
            continue;
        };
        let active = screen == app.screen;
        let style = if active {
            Style::default()
                .fg(Color::Rgb(0x1a, 0x1b, 0x26))
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(DIM)
        };
        tabs.push(Span::styled(format!(" {index} {} ", screen.title()), style));
        tabs.push(Span::styled(" ", Style::default()));
    }
    Paragraph::new(vec![status_line, Line::from(tabs)])
}

fn card(frame: &mut Frame, area: Rect, title: &str) -> Rect {
    let block = Block::bordered().border_style(Style::default().fg(BORDER));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return inner;
    }
    let title_area = Rect { height: 1, ..inner };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {title}"),
            Style::default().fg(DIM).add_modifier(Modifier::BOLD),
        ))),
        title_area,
    );
    Rect {
        y: inner.y + 1,
        height: inner.height - 1,
        ..inner
    }
}

fn overview_cards(app: &App, frame: &mut Frame, area: Rect) {
    let rows = Layout::vertical([Constraint::Length(10), Constraint::Min(0)]).split(area);
    let top =
        Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).split(rows[0]);
    let bottom =
        Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)]).split(rows[1]);
    battery_card(app, frame, top[0]);
    link_card(app, frame, top[1]);
    device_card(app, frame, bottom[0]);
    paired_card(app, frame, bottom[1]);
}

fn battery_card(app: &App, frame: &mut Frame, area: Rect) {
    let info = &app.status.info;
    let inner = card(frame, area, "BATTERY");
    let percent = info.battery_percent.unwrap_or(0);
    let charging = info.charging.as_deref().unwrap_or("unknown");
    let width = inner.width.saturating_sub(2) as usize;
    let (filled, rest) = meter(f32::from(percent) / 100.0, width);
    let lines = vec![
        Line::from(vec![
            Span::styled("󰂂 ", Style::default().fg(GREEN)),
            Span::styled(
                format!("{percent}%"),
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("   {charging}"), Style::default().fg(DIM)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(filled, Style::default().fg(GREEN)),
            Span::styled(rest, Style::default().fg(BG_HL)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("auto-off ", Style::default().fg(DIM)),
            Span::styled(
                info.auto_power_off_s
                    .map(|seconds| format!("{seconds} s"))
                    .unwrap_or_else(|| "?".to_owned()),
                Style::default().fg(FG),
            ),
            Span::styled("   on head ", Style::default().fg(DIM)),
            Span::styled(
                info.on_head_detection
                    .map(|on| on.to_string())
                    .unwrap_or_else(|| "?".to_owned()),
                Style::default().fg(FG),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn link_card(app: &App, frame: &mut Frame, area: Rect) {
    let info = &app.status.info;
    let inner = card(frame, area, "LINK");
    let audio = app.status.audio.as_ref();
    let codec = audio
        .and_then(|audio| audio.codec.clone())
        .map(|codec| codec_label(&codec))
        .or_else(|| info.codec.clone())
        .unwrap_or_else(|| "?".to_owned());
    let rate = audio
        .and_then(|audio| audio.sample_rate_hz)
        .or(info.sample_rate_hz);
    let profile = audio
        .map(|audio| audio.profile.clone())
        .unwrap_or_else(|| "?".to_owned());
    let state = audio.and_then(|audio| audio.state.clone());
    let lines = vec![
        Line::from(Span::styled(
            codec,
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(
                "{} · {}",
                rate.map(|hz| format!("{hz} Hz"))
                    .unwrap_or_else(|| "idle".to_owned()),
                audio
                    .and_then(|audio| audio.format.clone())
                    .unwrap_or_else(|| "?".to_owned())
            ),
            Style::default().fg(FG),
        )),
        Line::from(vec![
            Span::styled("profile ", Style::default().fg(DIM)),
            Span::styled(profile, Style::default().fg(FG)),
        ]),
        Line::from(vec![
            Span::styled("stream ", Style::default().fg(DIM)),
            Span::styled(
                state.unwrap_or_else(|| "?".to_owned()),
                Style::default().fg(
                    if audio.and_then(|a| a.state.clone()).as_deref() == Some("RUNNING") {
                        GREEN
                    } else {
                        DIM
                    },
                ),
            ),
        ]),
        Line::from(vec![
            Span::styled("autoswitch ", Style::default().fg(DIM)),
            Span::styled(
                audio
                    .and_then(|audio| audio.autoswitch)
                    .map(|enabled| enabled.to_string())
                    .unwrap_or_else(|| "?".to_owned()),
                Style::default().fg(FG),
            ),
        ]),
        if audio.is_some_and(|audio| audio.is_headset) {
            Line::from(Span::styled(
                "HFP active — press `a` for A2DP",
                Style::default().fg(YELLOW),
            ))
        } else {
            Line::from("")
        },
        match &app.status.now_playing {
            Some(track) => Line::from(Span::styled(
                format!("▶ {}", track.label()),
                Style::default().fg(if track.playing { FG } else { DIM }),
            )),
            None => Line::from(""),
        },
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn device_card(app: &App, frame: &mut Frame, area: Rect) {
    let info = &app.status.info;
    let inner = card(frame, area, "DEVICE");
    let lines = vec![
        kv("firmware", info.firmware.clone()),
        kv(
            "hardware",
            info.hardware_revision.map(|revision| revision.to_string()),
        ),
        kv("serial", info.serial.clone()),
        kv("gaia api", info.gaia_api.clone()),
        kv("physical", info.physical_state.clone()),
        kv("audio mode", info.audio_mode.clone()),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn paired_card(app: &App, frame: &mut Frame, area: Rect) {
    let inner = card(frame, area, "PAIRED");
    let mut lines = Vec::new();
    if app.status.info.paired.is_empty() {
        lines.push(Line::from(Span::styled("none", Style::default().fg(DIM))));
    }
    for device in &app.status.info.paired {
        let own = Some(device.index) == app.status.info.own_index;
        let style = if device.connected {
            Style::default().fg(FG)
        } else {
            Style::default().fg(DIM)
        };
        lines.push(Line::from(vec![
            Span::styled(
                if device.connected { "● " } else { "○ " },
                Style::default().fg(if device.connected { GREEN } else { DIM }),
            ),
            Span::styled(format!("[{:02}] ", device.index), Style::default().fg(DIM)),
            Span::styled(device.name.clone(), style),
            Span::styled(if own { "  *" } else { "" }, Style::default().fg(ACCENT)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn noise_cards(app: &App, frame: &mut Frame, area: Rect) {
    let rows = Layout::vertical([Constraint::Length(11), Constraint::Min(0)]).split(area);
    noise_card(app, frame, rows[0]);
    noise_hints(frame, rows[1]);
}

fn noise_card(app: &App, frame: &mut Frame, area: Rect) {
    let inner = card(frame, area, "NOISE CONTROL");
    let mode = app.anc_mode();
    let modes = [
        ("adaptive", AncMode::Adaptive),
        ("custom", AncMode::Custom),
        ("off", AncMode::Off),
    ];
    let mut segmented = vec![Span::styled(" mode  ", Style::default().fg(DIM))];
    for (label, value) in modes {
        let active = mode == value;
        let style = if active {
            Style::default()
                .fg(Color::Rgb(0x1a, 0x1b, 0x26))
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(DIM)
        };
        segmented.push(Span::styled(format!(" {label} "), style));
        segmented.push(Span::styled(" ", Style::default()));
    }
    segmented.push(Span::styled("  ←/→", Style::default().fg(DIM)));

    let level = app.status.info.transparency.unwrap_or(0);
    let width = inner.width.saturating_sub(24) as usize;
    let (filled, rest) = meter(f32::from(level) / 100.0, width.max(8));
    let transparency = Line::from(vec![
        Span::styled(
            " transparency  ",
            Style::default().fg(if app.selected == 1 { ACCENT } else { DIM }),
        ),
        Span::styled(filled, Style::default().fg(CYAN)),
        Span::styled(rest, Style::default().fg(BG_HL)),
        Span::styled(format!(" {level:>3}/100"), Style::default().fg(BRIGHT)),
    ]);

    let lines = vec![
        Line::from(segmented),
        Line::from(""),
        transparency,
        Line::from(Span::styled(
            "  0 = max ANC            50 = neutral            100 = open",
            Style::default().fg(DIM),
        )),
        Line::from(Span::styled(
            "  custom keeps the level stored on the headset",
            Style::default().fg(DIM),
        )),
        Line::from(""),
        kv(
            "anti-wind",
            Some(anti_wind_label(anti_wind(app)).to_owned()),
        ),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn noise_hints(frame: &mut Frame, area: Rect) {
    let inner = card(frame, area, "HOW IT BEHAVES");
    let lines = vec![
        Line::from(Span::styled(
            "adaptive   tracks the room, ignores the level",
            Style::default().fg(FG),
        )),
        Line::from(Span::styled(
            "custom     0 = max ANC, 50 = neutral, 100 = open",
            Style::default().fg(FG),
        )),
        Line::from(Span::styled(
            "off        no processing (firmware ≥ 2.13.32)",
            Style::default().fg(FG),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "←/→ changes the selected row; ↑/↓ picks it",
            Style::default().fg(DIM),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn sound_cards(app: &App, frame: &mut Frame, area: Rect) {
    let cols =
        Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).split(area);
    equalizer_card(app, frame, cols[0]);
    sound_settings(app, frame, cols[1]);
}

fn equalizer_card(app: &App, frame: &mut Frame, area: Rect) {
    let inner = card(frame, area, "EQUALIZER");
    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(inner);
    let gains = &app.status.info.eq_gains_db;
    frame.render_widget(
        EqBars {
            gains,
            selected: (app.selected < 5).then_some(app.selected),
            max_db: app
                .status
                .info
                .eq_range_db
                .map(|(min, _)| min.abs())
                .unwrap_or(6.0)
                .max(1.0),
        },
        rows[0],
    );
    let labels = ["63", "250", "1k", "4k", "8k"];
    let mut label_line = vec![Span::styled("    ", Style::default())];
    for (band, label) in labels.iter().enumerate() {
        let selected = app.selected == band;
        label_line.push(Span::styled(
            format!("{label:<5}"),
            Style::default()
                .fg(if selected { ACCENT } else { DIM })
                .add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ));
    }
    let mut value_line = vec![Span::styled("    ", Style::default())];
    for (band, _) in labels.iter().enumerate() {
        let gain = gains.get(band).copied().unwrap_or(0.0);
        let color = if app.selected == band {
            ACCENT
        } else if gain >= 0.0 {
            GREEN
        } else {
            ORANGE
        };
        value_line.push(Span::styled(
            format!("{gain:+.1} "),
            Style::default().fg(color),
        ));
    }
    frame.render_widget(
        Paragraph::new(vec![Line::from(label_line), Line::from(value_line)]),
        rows[1],
    );
}

fn sound_settings(app: &App, frame: &mut Frame, area: Rect) {
    let inner = card(frame, area, "SETTINGS");
    let bass = app.status.info.bass_boost.unwrap_or(false);
    let bass_style = if app.selected == 5 {
        Style::default().bg(BG_HL).fg(BRIGHT)
    } else {
        Style::default().fg(FG)
    };
    let marker = if app.selected == 5 { "▸" } else { " " };
    let lines = vec![
        Line::from(Span::styled(
            format!("{marker} bass boost   {}", if bass { "on " } else { "off" }),
            bass_style,
        )),
        Line::from(""),
        kv("audio mode", app.status.info.audio_mode.clone()),
        kv(
            "range",
            app.status
                .info
                .eq_range_db
                .map(|(min, max)| format!("{min:.1} … {max:+.1} dB")),
        ),
        kv(
            "preset",
            Some(match active_preset(app) {
                Some(name) => format!("● {name}"),
                None => "custom".to_owned(),
            }),
        ),
        Line::from(""),
        Line::from(Span::styled(
            "←/→ band · ↑/↓ level",
            Style::default().fg(DIM),
        )),
        Line::from(Span::styled(
            "step 0.5 dB · ↑/↓ toggles bass boost",
            Style::default().fg(DIM),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn devices_card(app: &App, frame: &mut Frame, area: Rect) {
    let inner = card(frame, area, "PAIRED DEVICES");
    app.hitboxes.borrow_mut().list = Some(inner);
    let mut lines = Vec::new();
    if app.status.info.paired.is_empty() {
        lines.push(Line::from(Span::styled(
            "no paired devices in the current snapshot",
            Style::default().fg(DIM),
        )));
    }
    for (row, device) in app.status.info.paired.iter().enumerate() {
        let own = Some(device.index) == app.status.info.own_index;
        let selected = app.selected == row;
        let style = if selected {
            Style::default()
                .bg(BG_HL)
                .fg(BRIGHT)
                .add_modifier(Modifier::BOLD)
        } else if device.connected {
            Style::default().fg(FG)
        } else {
            Style::default().fg(DIM)
        };
        lines.push(Line::from(Span::styled(
            format!(
                " {} [{:02}] {:<28} {}{}",
                if device.connected { "●" } else { "○" },
                device.index,
                device.name,
                if device.connected {
                    "connected"
                } else {
                    "disconnected"
                },
                if own { "  * this host" } else { "" }
            ),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " c connect · d disconnect · y/n confirm · * this host",
        Style::default().fg(DIM),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn diagnostics_cards(app: &App, frame: &mut Frame, area: Rect) {
    let cols =
        Layout::horizontal([Constraint::Percentage(46), Constraint::Percentage(54)]).split(area);
    let inner = card(frame, cols[0], "DIAGNOSTICS");
    let info = &app.status.info;
    let lines = vec![
        kv("firmware", info.firmware.clone()),
        kv(
            "hardware",
            info.hardware_revision.map(|revision| revision.to_string()),
        ),
        kv("serial", info.serial.clone()),
        kv("gaia api", info.gaia_api.clone()),
        kv("connected", Some(app.status.connected.to_string())),
        kv(
            "rfcomm",
            app.status.channel.map(|channel| channel.to_string()),
        ),
        kv("device error", app.status.error.clone()),
        kv("daemon error", app.daemon_error.clone()),
    ];
    frame.render_widget(Paragraph::new(lines), inner);

    let inner = card(frame, cols[1], "NOTIFICATIONS");
    let mut lines: Vec<Line> = Vec::new();
    if app.notifications.is_empty() {
        lines.push(Line::from(Span::styled(
            "nothing yet",
            Style::default().fg(DIM),
        )));
    }
    for entry in app.notifications.iter().rev().take(inner.height as usize) {
        lines.push(Line::from(Span::styled(
            entry.clone(),
            Style::default().fg(FG),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

// ------------------------------------------------------------------- minimal

fn minimal(app: &App, frame: &mut Frame) {
    *app.hitboxes.borrow_mut() = crate::app::Hitboxes::default();
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(6),
        Constraint::Length(1),
    ])
    .split(frame.area());
    let info = &app.status.info;
    let connection = if app.status.connected {
        match app.status.channel {
            Some(channel) => format!("connected · RFCOMM {channel}"),
            None => "connected".to_owned(),
        }
    } else {
        "headset not connected".to_owned()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(
                    " {}",
                    info.model
                        .clone()
                        .unwrap_or_else(|| "MOMENTUM 4".to_owned())
                ),
                Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · ", Style::default().fg(DIM)),
            Span::styled(
                info.battery_percent
                    .map(|percent| format!("{percent}%"))
                    .unwrap_or_else(|| "?".to_owned()),
                Style::default().fg(GREEN),
            ),
            Span::styled(" · ", Style::default().fg(DIM)),
            Span::styled(connection, Style::default().fg(FG)),
            Span::styled(" · ", Style::default().fg(DIM)),
            Span::styled(
                info.firmware.clone().unwrap_or_else(|| "?".to_owned()),
                Style::default().fg(DIM),
            ),
        ])),
        rows[0],
    );

    let lines = match app.screen {
        Screen::Overview => overview(app),
        Screen::Noise => noise(app),
        Screen::Sound => sound(app),
        Screen::Devices => devices(app),
        Screen::Diagnostics => diagnostics(app),
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title(format!(" {} ", app.screen.title())))
            .wrap(Wrap { trim: false }),
        rows[1],
    );
    frame.render_widget(footer(app, false), rows[2]);
}

// ------------------------------------------------------------------- widgets

struct EqBars<'a> {
    gains: &'a [f32],
    selected: Option<usize>,
    max_db: f32,
}

impl Widget for EqBars<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 3 || area.width < 20 {
            return;
        }
        let middle = area.y + area.height / 2;
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, middle)) {
                cell.set_char('─').set_style(Style::default().fg(BORDER));
            }
        }
        let band_width = area.width / 5;
        // Half-cell steps: one row is ~0.75 dB, too coarse to show a 0.5 dB move.
        let half = f32::from(area.height / 2).max(1.0) - 1.0;
        let subcells = half * 2.0;
        for band in 0..5usize {
            let gain = self.gains.get(band).copied().unwrap_or(0.0);
            let fraction = (gain / self.max_db).clamp(-1.0, 1.0);
            let length = (fraction.abs() * subcells).round() as u16;
            let center = area.x + band_width * band as u16 + band_width / 2;
            let color = if self.selected == Some(band) {
                ACCENT
            } else if fraction >= 0.0 {
                GREEN
            } else {
                ORANGE
            };
            if length == 0 {
                if let Some(cell) = buf.cell_mut((center, middle)) {
                    cell.set_char('▪').set_style(Style::default().fg(DIM));
                }
                continue;
            }
            let full = length / 2;
            let partial = length % 2 == 1;
            for step in 0..full {
                let y = if fraction >= 0.0 {
                    middle.saturating_sub(1 + step)
                } else {
                    middle + 1 + step
                };
                paint(buf, area, center, y, '█', color);
            }
            if partial {
                let y = if fraction >= 0.0 {
                    middle.saturating_sub(1 + full)
                } else {
                    middle + 1 + full
                };
                let glyph = if fraction >= 0.0 { '▄' } else { '▀' };
                paint(buf, area, center, y, glyph, color);
            }
        }
    }
}

fn paint(buf: &mut Buffer, area: Rect, center: u16, y: u16, glyph: char, color: Color) {
    if y < area.y || y >= area.y + area.height {
        return;
    }
    for dx in 0..3u16 {
        let x = center.saturating_sub(1) + dx;
        if x < area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(glyph).set_style(Style::default().fg(color));
            }
        }
    }
}

// ------------------------------------------------------------------- helpers

fn meter(fraction: f32, width: usize) -> (String, String) {
    let filled = (fraction.clamp(0.0, 1.0) * width as f32).round() as usize;
    ("█".repeat(filled), "░".repeat(width.saturating_sub(filled)))
}

fn codec_label(codec: &str) -> String {
    match codec {
        "sbc" => "SBC",
        "sbc_xq" => "SBC-XQ",
        "aac" => "AAC",
        "aptx" => "aptX",
        "aptx_hd" => "aptX HD",
        "aptx_ll" => "aptX LL",
        "ldac" => "LDAC",
        "cvsd" => "CVSD",
        "msbc" => "mSBC",
        other => other,
    }
    .to_owned()
}

fn kv(label: &str, value: Option<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {label:<14}"), Style::default().fg(DIM)),
        Span::styled(
            value.unwrap_or_else(|| "?".to_owned()),
            Style::default().fg(FG),
        ),
    ])
}

pub fn active_preset(app: &App) -> Option<String> {
    if app
        .status
        .info
        .audio_mode
        .as_deref()
        .is_some_and(|mode| mode.starts_with("off"))
    {
        return Some("neutral".to_owned());
    }
    m4::preset_for_gains(&app.status.info.eq_gains_db).map(str::to_owned)
}

fn anti_wind_label(value: &str) -> &str {
    match value {
        "0" => "off",
        "1" => "max",
        "2" => "auto",
        other => other,
    }
}

fn anti_wind(app: &App) -> &str {
    app.status
        .info
        .anc
        .as_deref()
        .and_then(|anc| {
            anc.split_whitespace()
                .find(|part| part.starts_with("anti-wind="))
                .and_then(|part| part.split('=').nth(1))
        })
        .unwrap_or("?")
}

fn help() -> Paragraph<'static> {
    Paragraph::new(vec![
        Line::from("q / Esc       quit and restore the terminal"),
        Line::from("Tab / Shift+Tab  next / previous screen"),
        Line::from("1..5          Overview / Noise / Sound / Devices / Diagnostics"),
        Line::from("Up / Down     select a row / change level on Sound"),
        Line::from("Left / Right  change value / pick EQ band on Sound"),
        Line::from("Sound screen: ←/→ walks the bands, ↑/↓ moves the bar"),
        Line::from("r             refresh status"),
        Line::from("m             switch layout: instrument / minimal"),
        Line::from("p             preset picker (Sound screen)"),
        Line::from("mouse         wheel changes the value, click picks a band/row"),
        Line::from("a             return from HFP to A2DP when offered"),
        Line::from("c / d         connect / disconnect selected paired device"),
        Line::from("y / n         confirm / cancel device action"),
        Line::from("?             close this help"),
        Line::from(""),
        Line::from("Writes are blocked while the headset is disconnected."),
    ])
    .block(Block::bordered().title(" Help "))
}

fn footer(app: &App, framed: bool) -> Paragraph<'static> {
    let text = if let Some(confirmation) = &app.confirmation {
        let action = match confirmation.action {
            DeviceAction::Connect => "connect",
            DeviceAction::Disconnect => "disconnect",
        };
        (
            format!("{action} {}? y = confirm, n = cancel", confirmation.name),
            YELLOW,
        )
    } else if !app.status.connected {
        (
            app.daemon_error
                .clone()
                .or_else(|| app.status.error.clone())
                .unwrap_or_else(|| "headset not connected; writes are blocked".to_owned()),
            RED,
        )
    } else {
        (
            {
                let keys = if app.screen == Screen::Sound {
                    "q quit · ←/→ band · ↑/↓ level · p presets · mouse · ? help"
                } else {
                    "q quit · Tab screens · ↑/↓ row · ←/→ change · m layout · ? help"
                };
                match app.message.as_deref() {
                    Some(message) if !message.is_empty() => format!(" {message} · {keys}"),
                    _ => format!(" {keys}"),
                }
            },
            DIM,
        )
    };
    let paragraph = Paragraph::new(Line::from(Span::styled(
        text.0,
        Style::default().fg(text.1),
    )));
    if framed {
        paragraph.block(Block::bordered().border_style(Style::default().fg(BORDER)))
    } else {
        paragraph
    }
}

// ------------------------------------------------------------ minimal sources

fn overview(app: &App) -> Vec<Line<'static>> {
    let info = &app.status.info;
    let mut lines = vec![
        field(
            "Battery",
            info.battery_percent.map(|percent| {
                format!(
                    "{percent}% ({})",
                    info.charging.as_deref().unwrap_or("unknown")
                )
            }),
        ),
        field("Firmware", info.firmware.clone()),
        field(
            "Codec",
            app.status
                .audio
                .as_ref()
                .and_then(|audio| audio.codec.clone())
                .map(|codec| codec_label(&codec))
                .or_else(|| info.codec.clone()),
        ),
        field(
            "Sample rate",
            app.status
                .audio
                .as_ref()
                .and_then(|audio| audio.sample_rate_hz)
                .or(info.sample_rate_hz)
                .map(|hz| format!("{hz} Hz")),
        ),
        field(
            "Profile",
            app.status.audio.as_ref().map(|audio| audio.profile.clone()),
        ),
        field("On head", info.on_head_detection.map(|on| on.to_string())),
        field("ANC", Some(format!("{:?}", app.anc_mode()).to_lowercase())),
        field(
            "Transparency",
            info.transparency.map(|level| format!("{level}/100")),
        ),
        field("GAIA API", info.gaia_api.clone()),
        field(
            "Playing",
            app.status.now_playing.as_ref().map(|track| track.label()),
        ),
    ];
    if app
        .status
        .audio
        .as_ref()
        .is_some_and(|audio| audio.is_headset)
    {
        lines.push(Line::from(Span::styled(
            "HFP active: press `a` to return to A2DP",
            Style::default().fg(YELLOW),
        )));
    }
    lines
}

fn noise(app: &App) -> Vec<Line<'static>> {
    vec![
        selected_row(
            app,
            0,
            "ANC mode",
            format!("{:?}", app.anc_mode()).to_lowercase(),
        ),
        selected_row(
            app,
            1,
            "Transparency",
            format!("{} / 100", app.status.info.transparency.unwrap_or(0)),
        ),
        selected_row(
            app,
            2,
            "Anti-wind (read-only)",
            anti_wind_label(anti_wind(app)).to_owned(),
        ),
        Line::from(""),
        Line::from(Span::styled(
            "←/→: mode cycles adaptive → custom → off",
            Style::default().fg(DIM),
        )),
        Line::from(Span::styled(
            "transparency step 5, custom mode only",
            Style::default().fg(DIM),
        )),
    ]
}

fn sound(app: &App) -> Vec<Line<'static>> {
    let frequencies = ["63 Hz", "250 Hz", "1 kHz", "4 kHz", "8 kHz"];
    let mut lines = frequencies
        .iter()
        .enumerate()
        .map(|(band, frequency)| {
            let gain = app
                .status
                .info
                .eq_gains_db
                .get(band)
                .copied()
                .unwrap_or(0.0);
            selected_row(
                app,
                band,
                format!("EQ {frequency}"),
                format!("{gain:+.1} dB"),
            )
        })
        .collect::<Vec<_>>();
    lines.push(selected_row(
        app,
        5,
        "Bass boost",
        if app.status.info.bass_boost.unwrap_or(false) {
            "on"
        } else {
            "off"
        }
        .to_owned(),
    ));
    lines.push(Line::from(""));
    lines.push(field("Audio mode", app.status.info.audio_mode.clone()));
    lines.push(field(
        "Range",
        app.status
            .info
            .eq_range_db
            .map(|(min, max)| format!("{min:.1} … {max:+.1} dB")),
    ));
    lines
}

fn devices(app: &App) -> Vec<Line<'static>> {
    let _ = app;
    if app.status.info.paired.is_empty() {
        return vec![Line::from("No paired devices in the current snapshot")];
    }
    let mut lines = app
        .status
        .info
        .paired
        .iter()
        .enumerate()
        .map(|(row, device)| {
            let own = if Some(device.index) == app.status.info.own_index {
                " *"
            } else {
                ""
            };
            let state = if device.connected {
                "connected"
            } else {
                "disconnected"
            };
            selected_row(
                app,
                row,
                format!("[{:02}] {}{}", device.index, device.name, own),
                state.to_owned(),
            )
        })
        .collect::<Vec<_>>();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "c: connect · d: disconnect · y/n: confirm · *: this host",
        Style::default().fg(DIM),
    )));
    lines
}

fn diagnostics(app: &App) -> Vec<Line<'static>> {
    let info = &app.status.info;
    let mut lines = vec![
        field("Firmware", info.firmware.clone()),
        field(
            "Hardware",
            info.hardware_revision.map(|revision| revision.to_string()),
        ),
        field("Serial", info.serial.clone()),
        field("GAIA API", info.gaia_api.clone()),
        field("Connected", Some(app.status.connected.to_string())),
        field(
            "RFCOMM",
            app.status.channel.map(|channel| channel.to_string()),
        ),
        field("Device error", app.status.error.clone()),
        field("Daemon error", app.daemon_error.clone()),
        Line::from(""),
        Line::from("Notifications (newest first):"),
    ];
    if app.notifications.is_empty() {
        lines.push(Line::from(Span::styled("  none", Style::default().fg(DIM))));
    } else {
        lines.extend(
            app.notifications
                .iter()
                .rev()
                .map(|entry| Line::from(format!("  {entry}"))),
        );
    }
    lines
}

fn field(label: &str, value: Option<String>) -> Line<'static> {
    Line::from(format!(
        "{label:<22} {}",
        value.unwrap_or_else(|| "?".to_owned())
    ))
}

fn selected_row(app: &App, row: usize, label: impl Into<String>, value: String) -> Line<'static> {
    let selected = app.selected == row;
    let label = label.into();
    let marker = if selected { ">" } else { " " };
    let style = if selected {
        Style::default().fg(YELLOW).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Line::from(Span::styled(format!("{marker} {label:<32} {value}"), style))
}
