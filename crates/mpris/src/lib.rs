//! "Now playing" through playerctl, which every MPRIS player supports.

use std::process::Command;

use m4::NowPlaying;

pub fn current() -> Option<NowPlaying> {
    let players = run(&["--list-all"])?;
    let mut paused = None;

    for player in players
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let status = run(&["-p", player, "status"]).unwrap_or_default();
        let playing = status.trim() == "Playing";
        if !playing && status.trim() != "Paused" {
            continue;
        }
        let metadata = run(&[
            "-p",
            player,
            "metadata",
            "--format",
            "{{artist}}\t{{title}}",
        ])
        .unwrap_or_default();
        let (artist, title) = split_metadata(&metadata);
        let track = NowPlaying {
            player: player.to_owned(),
            artist,
            title,
            playing,
        };
        if playing {
            return Some(track);
        }
        paused.get_or_insert(track);
    }
    paused
}

fn split_metadata(raw: &str) -> (Option<String>, Option<String>) {
    let mut parts = raw.trim_end_matches(['\n', '\r']).split('\t');
    let artist = parts
        .next()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    let title = parts
        .next()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    (artist, title)
}

fn playerctl() -> String {
    std::env::var("M4_PLAYERCTL").unwrap_or_else(|_| "playerctl".to_owned())
}

fn run(args: &[&str]) -> Option<String> {
    let output = Command::new(playerctl()).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_artist_and_title() {
        assert_eq!(
            split_metadata("Metallica\tOne\n"),
            (Some("Metallica".to_owned()), Some("One".to_owned()))
        );
    }

    #[test]
    fn current_reads_the_running_player() {
        use std::os::unix::fs::PermissionsExt;

        let script = std::env::temp_dir().join(format!("fake-playerctl-{}", std::process::id()));
        std::fs::write(
            &script,
            "#!/bin/sh\ncase \"$*\" in\n  *--list-all*) echo chromium.instance1 ;;\n  *status*) echo Playing ;;\n  *metadata*) printf 'Metallica\\tOne\\n' ;;\nesac\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("M4_PLAYERCTL", &script);

        let track = current().expect("track");
        assert_eq!(track.player, "chromium.instance1");
        assert_eq!(track.artist.as_deref(), Some("Metallica"));
        assert_eq!(track.title.as_deref(), Some("One"));
        assert!(track.playing);
        assert_eq!(track.label(), "Metallica — One");

        std::env::remove_var("M4_PLAYERCTL");
        let _ = std::fs::remove_file(&script);
    }

    #[test]
    fn tolerates_missing_fields_and_broken_output() {
        assert_eq!(split_metadata("\tOne"), (None, Some("One".to_owned())));
        assert_eq!(
            split_metadata("Metallica\t"),
            (Some("Metallica".to_owned()), None)
        );
        assert_eq!(split_metadata(""), (None, None));
        assert_eq!(
            split_metadata("No player could handle this command\n"),
            (Some("No player could handle this command".to_owned()), None)
        );
    }
}
