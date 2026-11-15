use std::process::ExitCode;

use audio::{Card, Error};

const USAGE: &str =
    "usage: audio status [--json] | profiles | set-profile <name> | autoswitch [on|off]";

enum Command {
    Status { json: bool },
    Profiles,
    SetProfile { name: String },
    Autoswitch { state: Option<bool> },
    Help,
}

fn main() -> ExitCode {
    let result = parse_args().and_then(run);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("audio: {error}");
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args() -> Result<Command, Error> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        return Ok(Command::Help);
    };
    match command.as_str() {
        "status" => {
            let json = match args.next().as_deref() {
                Some("--json") => true,
                Some(argument) => {
                    return Err(argument_error(format!(
                        "unknown status argument {argument:?}"
                    )))
                }
                None => false,
            };
            ensure_finished(args)?;
            Ok(Command::Status { json })
        }
        "profiles" => {
            ensure_finished(args)?;
            Ok(Command::Profiles)
        }
        "set-profile" => {
            let name = args
                .next()
                .ok_or_else(|| argument_error("missing profile name".to_owned()))?;
            ensure_finished(args)?;
            Ok(Command::SetProfile { name })
        }
        "autoswitch" => {
            let state = match args.next().as_deref() {
                Some("on") => Some(true),
                Some("off") => Some(false),
                Some(argument) => {
                    return Err(argument_error(format!(
                        "expected on or off, got {argument:?}"
                    )))
                }
                None => None,
            };
            ensure_finished(args)?;
            Ok(Command::Autoswitch { state })
        }
        "--help" | "-h" => Ok(Command::Help),
        _ => Err(argument_error(format!("unknown command {command:?}"))),
    }
}

fn ensure_finished(mut args: impl Iterator<Item = String>) -> Result<(), Error> {
    match args.next() {
        Some(argument) => Err(argument_error(format!("unexpected argument {argument:?}"))),
        None => Ok(()),
    }
}

fn argument_error(detail: String) -> Error {
    Error::Parse {
        command: "audio arguments".to_owned(),
        detail,
    }
}

fn run(command: Command) -> Result<(), Error> {
    match command {
        Command::Help => println!("{USAGE}"),
        Command::Status { json } => {
            let card = require_headset()?;
            if json {
                let output = serde_json::to_string_pretty(&card).map_err(|error| Error::Parse {
                    command: "audio status --json".to_owned(),
                    detail: error.to_string(),
                })?;
                println!("{output}");
            } else {
                print_status(&card);
            }
        }
        Command::Profiles => {
            let card = require_headset()?;
            println!("{} ({})", card.description, card.name);
            for profile in card.profiles {
                let active = if profile.name == card.active_profile {
                    '*'
                } else {
                    ' '
                };
                println!(
                    "{active} {:<28} {:<70} available={} sinks={} sources={} priority={}",
                    profile.name,
                    profile.description,
                    profile.available,
                    profile.sinks,
                    profile.sources,
                    profile.priority
                );
            }
        }
        Command::SetProfile { name } => {
            let card = require_headset()?;
            let profile = card
                .profiles
                .iter()
                .find(|profile| profile.name == name)
                .ok_or_else(|| Error::Parse {
                    command: "audio set-profile".to_owned(),
                    detail: format!("unknown profile {name:?}; run `audio profiles`"),
                })?;
            if !profile.available {
                return Err(Error::Parse {
                    command: "audio set-profile".to_owned(),
                    detail: format!("profile {name:?} is unavailable"),
                });
            }
            audio::set_profile(&card.name, &name)?;
            println!("{}: {name}", card.description);
        }
        Command::Autoswitch { state } => {
            if let Some(enabled) = state {
                audio::set_autoswitch_headset_profile(enabled)?;
            }
            match audio::autoswitch_headset_profile()? {
                Some(enabled) => println!("{enabled}"),
                None => println!("unknown"),
            }
        }
    }
    Ok(())
}

fn require_headset() -> Result<Card, Error> {
    audio::headset()?.ok_or_else(|| Error::Parse {
        command: "audio::headset".to_owned(),
        detail: "MOMENTUM 4 was not found among PipeWire cards".to_owned(),
    })
}

fn print_status(card: &Card) {
    println!("card: {} ({})", card.description, card.name);
    println!("address: {}", card.address);
    println!("profile: {}", card.active_profile);
    println!("codec: {}", card.codec().unwrap_or("unknown"));
    match card.sink.as_ref() {
        Some(sink) => {
            println!(
                "sample_rate: {}",
                sink.sample_rate_hz
                    .map_or_else(|| "unknown".to_owned(), |rate| format!("{rate}"))
            );
            println!("format: {}", sink.format.as_deref().unwrap_or("unknown"));
            println!(
                "channels: {}",
                sink.channels
                    .map_or_else(|| "unknown".to_owned(), |channels| channels.to_string())
            );
            println!("state: {}", sink.state);
        }
        None => {
            println!("sample_rate: unknown");
            println!("format: unknown");
            println!("channels: unknown");
            println!("state: no sink");
        }
    }
}
