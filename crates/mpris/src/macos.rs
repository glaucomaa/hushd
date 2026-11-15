use std::process::Command;

use m4::NowPlaying;

const PLAYERS: &str = r#"
function current(name) {
    try {
        const app = Application(name);
        if (!app.running()) return null;
        const state = app.playerState();
        if (state !== "playing" && state !== "paused") return null;
        const track = app.currentTrack;
        return {
            player: name,
            artist: track.artist(),
            title: track.name(),
            playing: state === "playing"
        };
    } catch (_) {
        return null;
    }
}
JSON.stringify(current("Music") || current("Spotify"));
"#;

pub fn current() -> Option<NowPlaying> {
    let output = Command::new("osascript")
        .arg("-l")
        .arg("JavaScript")
        .arg("-e")
        .arg(PLAYERS)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()?
}
