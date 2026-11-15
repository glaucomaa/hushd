//! Renders one frame of the TUI to stdout without needing a terminal.
//!
//! `cargo run -p tui --example snapshot -- --screen 2 --layout minimal`

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use tui::app::{App, AppLayout, Screen};

fn main() {
    let mut layout = AppLayout::Instrument;
    let mut screen = 1u8;
    let mut width = 120u16;
    let mut json = false;
    let mut presets = false;
    let mut height = 40u16;
    let mut socket = std::env::var_os("M4D_SOCKET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(ipc::socket_path);

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--minimal" => layout = AppLayout::Minimal,
            "--json" => json = true,
            "--presets" => presets = true,
            "--screen" => {
                screen = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1)
            }
            "--width" => {
                width = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(120)
            }
            "--height" => {
                height = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(40)
            }
            "--socket" => {
                if let Some(value) = args.next() {
                    socket = std::path::PathBuf::from(value);
                }
            }
            other => {
                eprintln!("unknown argument {other:?}");
                std::process::exit(2);
            }
        }
    }

    let mut app = App::with_layout(layout);
    match ipc::request(&socket, &ipc::Request::Status) {
        Ok(ipc::Response::Status(status)) => app.status = status,
        Ok(other) => eprintln!("unexpected reply: {other:?}"),
        Err(error) => eprintln!("daemon: {error}"),
    }
    app.screen = Screen::from_number(screen).unwrap_or(Screen::Overview);
    app.show_presets = presets;

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| tui::render::render(&app, frame))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    if json {
        let mut cells = Vec::new();
        for row in 0..buffer.area.height {
            for column in 0..buffer.area.width {
                let index = usize::from(row) * usize::from(buffer.area.width) + usize::from(column);
                let cell = &buffer.content()[index];
                cells.push(serde_json::json!({
                    "c": cell.symbol(),
                    "fg": color(cell.fg),
                    "bg": color(cell.bg),
                    "b": cell.modifier.contains(ratatui::style::Modifier::BOLD),
                }));
            }
        }
        println!(
            "{}",
            serde_json::json!({
                "width": buffer.area.width,
                "height": buffer.area.height,
                "cells": cells,
            })
        );
        return;
    }
    let mut text = String::new();
    for row in 0..buffer.area.height {
        for column in 0..buffer.area.width {
            let index = usize::from(row) * usize::from(buffer.area.width) + usize::from(column);
            text.push_str(buffer.content()[index].symbol());
        }
        text.push('\n');
    }
    print!("{text}");
}

fn color(value: ratatui::style::Color) -> String {
    match value {
        ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        ratatui::style::Color::Reset => "reset".to_owned(),
        ratatui::style::Color::Black => "#1a1b26".to_owned(),
        ratatui::style::Color::Red => "#f7768e".to_owned(),
        ratatui::style::Color::Green => "#9ece6a".to_owned(),
        ratatui::style::Color::Yellow => "#e0af68".to_owned(),
        ratatui::style::Color::Blue => "#7aa2f7".to_owned(),
        ratatui::style::Color::Magenta => "#bb9af7".to_owned(),
        ratatui::style::Color::Cyan => "#7dcfff".to_owned(),
        ratatui::style::Color::Gray => "#a9b1d6".to_owned(),
        ratatui::style::Color::White => "#c0caf5".to_owned(),
        other => format!("{other:?}"),
    }
}
