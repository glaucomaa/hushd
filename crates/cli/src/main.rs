use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Toggle {
    On,
    Off,
}
use ipc::{AncMode, Event, Request, Response};

#[derive(Parser)]
#[command(
    name = "m4",
    version,
    about = "Sennheiser MOMENTUM 4 control through the m4d daemon"
)]
struct Cli {
    /// Override the daemon socket path
    #[arg(long, global = true)]
    socket: Option<PathBuf>,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Headset state as the daemon sees it
    Status,
    /// Stream state changes until interrupted
    Watch,
    /// Stream Waybar JSON lines, one per state change
    Waybar,
    /// Connect the paired headset through the system Bluetooth backend
    Connect,
    /// Disconnect the headset through the system Bluetooth backend
    Disconnect,
    /// Noise control: adaptive, custom, off or cycle
    Anc { mode: String },
    /// Transparency level 0..100, or a relative step such as +5 / -5 (custom mode)
    Transparency {
        #[arg(allow_hyphen_values = true)]
        level: String,
    },
    /// Set one EQ band in dB, e.g. `m4 eq 0 -2.5`
    Eq {
        band: u8,
        #[arg(allow_hyphen_values = true)]
        db: f32,
    },
    /// Sennheiser EQ preset: list | rock | pop | dance | hip-hop | classical | movie | jazz | neutral
    Preset { name: String },
    /// Bass boost on/off
    BassBoost { state: Toggle },
    /// Lock or unlock the touch panel
    TouchLock { state: Toggle },
    /// Audio details; PipeWire profile switching is Linux-only
    Audio { profile: Option<String> },
    /// Read one property
    Get { property: String },
    /// Health check through the running daemon (no second RFCOMM link)
    Doctor,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("m4: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let socket = cli.socket.clone().unwrap_or_else(ipc::socket_path);
    let json = cli.json;

    let request = match cli.command {
        None | Some(Command::Status) => {
            let response = send(&socket, &Request::Status)?;
            match response {
                Response::Status(status) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&status).map_err(|e| e.to_string())?
                        );
                    } else {
                        let state = if status.connected {
                            match status.channel {
                                Some(channel) => format!("connected (rfcomm channel {channel})"),
                                None => "connected".to_owned(),
                            }
                        } else {
                            match status.error.as_deref() {
                                Some(reason) => format!("disconnected: {reason}"),
                                None => "disconnected".to_owned(),
                            }
                        };
                        println!("{state}");
                        if status.connected {
                            print!("{}", status.info.to_text());
                            if let Some(track) = &status.now_playing {
                                println!("playing            {}", track.label());
                            }
                            if let Some(audio) = &status.audio {
                                println!(
                                    "audio              {} · {} · {}",
                                    audio.profile,
                                    audio.codec.clone().unwrap_or_else(|| "?".into()),
                                    audio
                                        .sample_rate_hz
                                        .map(|hz| format!("{hz} Hz"))
                                        .unwrap_or_else(|| "idle".into()),
                                );
                            }
                        }
                    }
                    return Ok(());
                }
                other => return Err(expect_ok(other)),
            }
        }
        Some(Command::Waybar) => {
            let mut last = String::new();
            ipc::watch(&socket, |event| {
                if let Event::State { status } = event {
                    let line = waybar_line(&status);
                    if line != last {
                        println!("{line}");
                        last = line;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
            return Ok(());
        }
        Some(Command::Watch) => {
            ipc::watch(&socket, |event| match (&event, json) {
                (Event::State { status }, false) if status.connected => {
                    print!("{}", status.info.to_text());
                }
                (Event::Connected { channel }, false) => {
                    println!("connected (rfcomm channel {channel})")
                }
                (Event::Disconnected { reason }, false) => println!("disconnected: {reason}"),
                (Event::Notification { command, payload }, false) => {
                    println!("notification {command:#06x} {payload:02x?}");
                }
                (_, true) => match serde_json::to_string(&event) {
                    Ok(line) => println!("{line}"),
                    Err(error) => eprintln!("m4: {error}"),
                },
                (Event::State { status }, false) => {
                    println!("waiting: {}", status.error.as_deref().unwrap_or("no data"));
                }
            })
            .map_err(|error| error.to_string())?;
            return Ok(());
        }
        Some(Command::Connect) => Request::HeadsetConnect { connect: true },
        Some(Command::Disconnect) => Request::HeadsetConnect { connect: false },
        Some(Command::Anc { mode }) => {
            let mode = match mode.as_str() {
                "adaptive" => AncMode::Adaptive,
                "custom" => AncMode::Custom,
                "off" => AncMode::Off,
                "cycle" => match current_mode(&socket)? {
                    AncMode::Adaptive => AncMode::Custom,
                    AncMode::Custom => AncMode::Off,
                    AncMode::Off => AncMode::Adaptive,
                },
                other => {
                    return Err(format!(
                        "unknown mode {other}; expected adaptive, custom, off or cycle"
                    ))
                }
            };
            Request::Anc { mode }
        }
        Some(Command::Transparency { level }) => {
            let trimmed = level.trim();
            let value: i16 = trimmed
                .parse()
                .map_err(|_| format!("transparency needs a number, got {trimmed:?}"))?;
            let relative = trimmed.starts_with('+') || trimmed.starts_with('-');
            let level = if relative {
                let current = i16::from(current_status(&socket)?.info.transparency.unwrap_or(0));
                (current + value).clamp(0, 100) as u8
            } else {
                if !(0..=100).contains(&value) {
                    return Err("transparency level must be 0..100".to_owned());
                }
                value as u8
            };
            Request::Transparency { level }
        }
        Some(Command::Eq { band, db }) => {
            if band > 4 {
                return Err("MOMENTUM 4 has 5 EQ bands, 0..4".to_owned());
            }
            if !db.is_finite() || !(-6.0..=6.0).contains(&db) {
                return Err("EQ gain must be a finite value within ±6 dB".to_owned());
            }
            let centibel = (db * 10.0).round() as i16;
            Request::EqGain { band, centibel }
        }
        Some(Command::Preset { name }) if name == "list" => {
            println!(
                "{:<10} {:>6} {:>6} {:>6} {:>6} {:>6}",
                "preset", "63", "250", "1k", "4k", "8k"
            );
            for preset in m4::PRESETS {
                let gains: Vec<String> = preset
                    .gains_db
                    .iter()
                    .map(|gain| format!("{gain:+.1}"))
                    .collect();
                println!(
                    "{:<10} {:>6} {:>6} {:>6} {:>6} {:>6}",
                    preset.name, gains[0], gains[1], gains[2], gains[3], gains[4]
                );
            }
            return Ok(());
        }
        Some(Command::Preset { name }) => Request::Preset { name },
        Some(Command::BassBoost { state }) => Request::BassBoost {
            enabled: state.is_on(),
        },
        Some(Command::TouchLock { state }) => Request::TouchLock {
            enabled: state.is_on(),
        },
        Some(Command::Audio { profile }) => {
            let response = send(
                &socket,
                &Request::Audio {
                    set_profile: profile,
                },
            )?;
            match response {
                Response::Audio(state) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?
                        );
                    } else {
                        println!("card          {}", state.card);
                        println!("address       {}", state.address);
                        println!("profile       {}", state.profile);
                        println!(
                            "codec         {}",
                            state.codec.unwrap_or_else(|| "?".into())
                        );
                        println!(
                            "sample rate   {}",
                            state
                                .sample_rate_hz
                                .map(|hz| format!("{hz} Hz"))
                                .unwrap_or_else(|| "?".into())
                        );
                        println!(
                            "format        {}",
                            state.format.unwrap_or_else(|| "?".into())
                        );
                        println!(
                            "state         {}",
                            state.state.unwrap_or_else(|| "?".into())
                        );
                        println!(
                            "autoswitch    {}",
                            match state.autoswitch {
                                Some(value) => value.to_string(),
                                None => "?".to_owned(),
                            }
                        );
                    }
                    return Ok(());
                }
                other => return Err(expect_ok(other)),
            }
        }
        Some(Command::Get { property }) => Request::Get { property },
        Some(Command::Doctor) => return doctor(&socket),
    };

    match send(&socket, &request)? {
        Response::Ok => {
            if !json {
                println!("ok");
            }
            Ok(())
        }
        Response::Value { value } => {
            println!("{value}");
            Ok(())
        }
        other => Err(expect_ok(other)),
    }
}

impl Toggle {
    fn is_on(self) -> bool {
        matches!(self, Self::On)
    }
}

fn current_status(socket: &std::path::Path) -> Result<ipc::Status, String> {
    match send(socket, &Request::Status)? {
        Response::Status(status) => Ok(status),
        other => Err(expect_ok(other)),
    }
}

fn current_mode(socket: &std::path::Path) -> Result<AncMode, String> {
    let status = current_status(socket)?;
    if status.info.anc_status == Some(0) {
        return Ok(AncMode::Off);
    }
    Ok(match status.info.anc.as_deref() {
        Some(anc) if anc.contains("adaptive=1") => AncMode::Adaptive,
        _ => AncMode::Custom,
    })
}

fn doctor(socket: &std::path::Path) -> Result<(), String> {
    let mut failed = 0usize;
    let mut report = |ok: bool, label: &str, detail: String| {
        let mark = if ok { "ok  " } else { "FAIL" };
        println!("  {mark} {label:<14} {detail}");
        if !ok {
            failed += 1;
        }
    };

    println!("daemon");
    let status = current_status(socket)?;
    report(true, "socket", format!("{}", socket.display()));

    println!("headset");
    report(
        status.connected,
        "connected",
        match (&status.error, status.channel) {
            (_, Some(channel)) if status.connected => format!("RFCOMM {channel}"),
            (Some(error), _) => error.clone(),
            _ => "not connected".to_owned(),
        },
    );
    if status.connected {
        let info = &status.info;
        report(
            info.firmware.is_some(),
            "identity",
            format!(
                "{} · firmware {} · battery {}%",
                info.model.clone().unwrap_or_else(|| "?".to_owned()),
                info.firmware.clone().unwrap_or_else(|| "?".to_owned()),
                info.battery_percent
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "?".to_owned()),
            ),
        );
        report(
            info.eq_gains_db.len() == 5,
            "eq",
            format!(
                "{} bands, gains {:?}",
                info.eq_gains_db.len(),
                info.eq_gains_db
            ),
        );
    }

    println!("audio");
    #[cfg(target_os = "linux")]
    match send(socket, &Request::Audio { set_profile: None })? {
        Response::Audio(audio) => {
            report(
                audio.is_headset || audio.profile.starts_with("a2dp"),
                "pipewire",
                format!(
                    "{} · {} · {}",
                    audio.profile,
                    audio.codec.clone().unwrap_or_else(|| "?".to_owned()),
                    audio
                        .sample_rate_hz
                        .map(|hz| format!("{hz} Hz"))
                        .unwrap_or_else(|| "idle".to_owned())
                ),
            );
        }
        Response::Error { message } => report(false, "pipewire", message),
        other => report(false, "pipewire", format!("unexpected reply: {other:?}")),
    }
    #[cfg(not(target_os = "linux"))]
    report(
        status.connected && status.info.codec.is_some(),
        "gaia",
        format!(
            "{} · {}",
            status.info.codec.clone().unwrap_or_else(|| "?".into()),
            status
                .info
                .sample_rate_hz
                .map(|hz| format!("{hz} Hz"))
                .unwrap_or_else(|| "idle".into())
        ),
    );

    println!();
    if failed == 0 {
        println!("all checks passed");
        Ok(())
    } else {
        Err(format!("{failed} check(s) failed"))
    }
}

fn waybar_line(status: &ipc::Status) -> String {
    if !status.connected {
        let reason = status
            .error
            .clone()
            .unwrap_or_else(|| "disconnected".to_owned());
        return serde_json::json!({
            "text": "󰋋 off",
            "tooltip": format!("MOMENTUM 4: {reason}"),
            "class": "offline",
        })
        .to_string();
    }

    let percent = status.info.battery_percent.unwrap_or(0);
    let icon = match percent {
        80..=100 => "󰂂",
        60..=79 => "󰂁",
        40..=59 => "󰂀",
        20..=39 => "󰁾",
        _ => "󰁻",
    };
    let mode = if status.info.anc_status == Some(0) {
        "off".to_owned()
    } else if status
        .info
        .anc
        .as_deref()
        .is_some_and(|anc| anc.contains("adaptive=1"))
    {
        "adaptive".to_owned()
    } else {
        format!("custom {}", status.info.transparency.unwrap_or(0))
    };
    let codec = status
        .audio
        .as_ref()
        .and_then(|audio| audio.codec.clone())
        .map(|codec| match codec.as_str() {
            "aptx_hd" => "aptX HD".to_owned(),
            "aptx" => "aptX".to_owned(),
            "sbc" => "SBC".to_owned(),
            "sbc_xq" => "SBC-XQ".to_owned(),
            "aac" => "AAC".to_owned(),
            "ldac" => "LDAC".to_owned(),
            "cvsd" => "CVSD".to_owned(),
            "msbc" => "mSBC".to_owned(),
            other => other.to_owned(),
        })
        .or_else(|| status.info.codec.clone())
        .unwrap_or_else(|| "?".to_owned());
    let text = format!("{icon} {percent}% · {mode}");
    let tooltip = format!(
        "MOMENTUM 4\nbattery: {percent}% ({})\nnoise: {mode}\ncodec: {codec}",
        status
            .info
            .charging
            .clone()
            .unwrap_or_else(|| "unknown".to_owned())
    );
    serde_json::json!({
        "text": text,
        "tooltip": tooltip,
        "class": mode.split(' ').next().unwrap_or("custom"),
    })
    .to_string()
}

fn send(socket: &std::path::Path, request: &Request) -> Result<Response, String> {
    ipc::request(socket, request).map_err(|error| match &error {
        ipc::Error::Io { path, source }
            if matches!(
                source.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            let supervisor = if cfg!(target_os = "macos") {
                "launchctl kickstart -k gui/$(id -u)/dev.m4.m4d"
            } else {
                "systemctl --user start m4d"
            };
            format!(
                "m4d is not running (socket {}); start it with `m4d` or `{supervisor}`",
                path.display()
            )
        }
        other => other.to_string(),
    })
}

fn expect_ok(response: Response) -> String {
    match response {
        Response::Error { message } => message,
        other => format!("unexpected reply: {other:?}"),
    }
}
