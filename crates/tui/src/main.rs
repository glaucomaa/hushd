use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use tui::app::{App, AppLayout, Key};
use tui::render::render;
use tui::DaemonBackend;

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableMouseCapture,
            LeaveAlternateScreen,
            Show
        );
    }
}

const USAGE: &str = "usage: m4-tui [--minimal | --instrument]";

fn main() -> io::Result<()> {
    let mut layout = AppLayout::Instrument;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--minimal" | "-m" => layout = AppLayout::Minimal,
            "--instrument" => layout = AppLayout::Instrument,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => {
                eprintln!("m4-tui: unknown argument {other:?}\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let socket = std::env::var_os("M4D_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(ipc::socket_path);
    let mut backend = DaemonBackend::new(socket);
    let _guard = TerminalGuard::enter()?;
    let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    run(terminal, &mut backend, layout)
}

fn run(
    mut terminal: Terminal<CrosstermBackend<io::Stdout>>,
    backend: &mut DaemonBackend,
    layout: AppLayout,
) -> io::Result<()> {
    let mut app = App::with_layout(layout);
    app.refresh(backend);
    let mut refreshed = Instant::now();

    loop {
        for event in backend.drain_events() {
            app.apply_event(event);
        }
        app.tick(backend);
        terminal.draw(|frame| render(&app, frame))?;
        if app.should_quit {
            app.flush(backend);
            return Ok(());
        }

        if event::poll(Duration::from_millis(30))? {
            match event::read()? {
                Event::Key(event) => {
                    if matches!(event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                        if let Some(key) = map_key(event.code) {
                            app.handle_key(key, backend);
                        }
                    }
                }
                Event::Mouse(event) => {
                    let mouse = match event.kind {
                        MouseEventKind::ScrollUp => Some(tui::app::Mouse::ScrollUp),
                        MouseEventKind::ScrollDown => Some(tui::app::Mouse::ScrollDown),
                        MouseEventKind::Down(MouseButton::Left) => Some(tui::app::Mouse::Click {
                            column: event.column,
                            row: event.row,
                        }),
                        _ => None,
                    };
                    if let Some(mouse) = mouse {
                        app.handle_mouse(mouse, backend);
                    }
                }
                _ => {}
            }
        }

        if refreshed.elapsed() >= Duration::from_secs(1) {
            app.refresh(backend);
            refreshed = Instant::now();
        }
    }
}

fn map_key(code: KeyCode) -> Option<Key> {
    match code {
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::Left => Some(Key::Left),
        KeyCode::Right => Some(Key::Right),
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::BackTab => Some(Key::BackTab),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Esc => Some(Key::Esc),
        KeyCode::Char(character) => match character {
            '1'..='5' => Some(Key::Number(character as u8 - b'0')),
            other => Some(Key::Char(other)),
        },
        _ => None,
    }
}
