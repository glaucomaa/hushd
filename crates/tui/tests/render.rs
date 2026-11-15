use ratatui::backend::TestBackend;
use ratatui::Terminal;

use tui::app::{App, AppLayout, Key, Screen};
use tui::MockBackend;

fn app_with(layout: AppLayout) -> App {
    let mut backend = MockBackend::new();
    let mut app = App::with_layout(layout);
    app.refresh(&mut backend);
    app
}

fn draw(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| tui::render::render(app, frame))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for row in 0..buffer.area.height {
        for column in 0..buffer.area.width {
            let index = usize::from(row) * usize::from(buffer.area.width) + usize::from(column);
            text.push_str(buffer.content()[index].symbol());
        }
        text.push('\n');
    }
    text
}

#[test]
fn instrument_layout_shows_cards() {
    let app = app_with(AppLayout::Instrument);
    let screen = draw(&app, 120, 40);
    for expected in ["BATTERY", "LINK", "DEVICE", "PAIRED", "aptX HD"] {
        assert!(
            screen.contains(expected),
            "missing {expected} in:\n{screen}"
        );
    }
}

#[test]
fn minimal_layout_shows_plain_fields() {
    let app = app_with(AppLayout::Minimal);
    let screen = draw(&app, 80, 24);
    for expected in ["Battery", "Firmware", "Codec"] {
        assert!(
            screen.contains(expected),
            "missing {expected} in:\n{screen}"
        );
    }
    assert!(!screen.contains("BATTERY"), "minimal should not draw cards");
}

#[test]
fn m_toggles_layout() {
    let mut backend = MockBackend::new();
    let mut app = App::with_layout(AppLayout::Instrument);
    app.handle_key(Key::Char('m'), &mut backend);
    assert_eq!(app.layout, AppLayout::Minimal);
    app.handle_key(Key::Char('m'), &mut backend);
    assert_eq!(app.layout, AppLayout::Instrument);
}

#[test]
fn every_screen_renders_in_both_layouts() {
    for layout in [AppLayout::Instrument, AppLayout::Minimal] {
        for number in 1..=5u8 {
            let mut app = app_with(layout);
            app.screen = Screen::from_number(number).expect("screen");
            for (width, height) in [(120u16, 40u16), (80, 24)] {
                let screen = draw(&app, width, height);
                assert!(
                    !screen.trim().is_empty(),
                    "{layout:?} {number} {width}x{height}"
                );
                assert!(
                    screen.contains(app.screen.title()),
                    "{layout:?} screen {number} at {width}x{height} lost its title:\n{screen}"
                );
            }
        }
    }
}

#[test]
fn help_overlay_renders() {
    let mut app = app_with(AppLayout::Instrument);
    app.show_help = true;
    let screen = draw(&app, 100, 30);
    assert!(screen.contains("Help"));
    assert!(screen.contains("switch layout"));
}
