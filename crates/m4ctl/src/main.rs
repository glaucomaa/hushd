use std::process::ExitCode;

use clap::{Parser, Subcommand};
use gaia::Frame;
use m4::{Info, Session};

#[derive(Parser)]
#[command(
    name = "m4ctl",
    version,
    about = "Sennheiser MOMENTUM 4 control over RFCOMM/GAIA"
)]
struct Cli {
    /// Headset MAC; discovered through BlueZ when omitted
    #[arg(long, global = true)]
    address: Option<String>,
    /// RFCOMM channel override (both GAIA channels answer on firmware 3.38.3)
    #[arg(long, global = true)]
    channel: Option<u8>,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Headset state snapshot
    Info,
    /// Bluetooth devices known to BlueZ
    Devices,
    /// End-to-end health check: BlueZ, PipeWire, GAIA
    Doctor,
    /// One property by name
    Get { property: String },
    /// Raw hex request: vendor, command and optional payload
    #[command(hide = true)]
    Raw {
        #[arg(value_parser = parse_hex)]
        bytes: Vec<u8>,
        /// Raw frames bypass every guard; dangerous commands stay blocked anyway
        #[arg(long = "unsafe")]
        unsafe_flag: bool,
    },
}

struct Peer {
    name: String,
    address: String,
    connected: bool,
    adapter: String,
}

#[cfg(target_os = "linux")]
fn peers() -> Result<Vec<Peer>, String> {
    rfcomm::bluez::devices()
        .map(|devices| {
            devices
                .into_iter()
                .map(|device| Peer {
                    name: device.name,
                    address: device.address,
                    connected: device.connected,
                    adapter: device.adapter,
                })
                .collect()
        })
        .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "linux"))]
fn peers() -> Result<Vec<Peer>, String> {
    Err("BlueZ discovery is Linux-only; pass --address".to_owned())
}

fn parse_hex(text: &str) -> Result<u8, String> {
    u8::from_str_radix(text, 16).map_err(|_| format!("not a hex byte: {text}"))
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("m4ctl: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(Command::Doctor) = cli.command {
        return doctor();
    }

    if let Some(Command::Raw { bytes, unsafe_flag }) = &cli.command {
        validate_raw(bytes, *unsafe_flag)?;
    }

    if let Some(Command::Devices) = cli.command {
        let devices = peers().map_err(|error| error.to_string())?;
        for device in devices {
            let state = if device.connected {
                "connected"
            } else {
                "disconnected"
            };
            println!(
                "{:<18} {:<12} {:<20} {}",
                device.address, state, device.name, device.adapter
            );
        }
        return Ok(());
    }

    let address = match cli.address {
        Some(text) => m4::parse_address(&text)?,
        None => {
            let device = peers()
                .map_err(|error| error.to_string())?
                .into_iter()
                .find(|peer| peer.name.to_uppercase().contains("MOMENTUM 4"))
                .ok_or("no MOMENTUM 4 known to BlueZ; pass --address")?;
            if !device.connected {
                return Err(format!("{} is not connected", device.name).into());
            }
            m4::parse_address(&device.address)?
        }
    };

    let channels = cli.channel.as_ref().map(std::slice::from_ref);
    let mut session = open_session(address, channels)?;

    match cli.command {
        None | Some(Command::Info) => {
            let info = Info::read(&mut session);
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                print!("{}", info.to_text());
            }
        }
        Some(Command::Get { property }) => {
            let value = get(&mut session, &property)?;
            if cli.json {
                println!("{{\"{}\": {}}}", property, serde_json::to_string(&value)?);
            } else {
                println!("{value}");
            }
        }
        Some(Command::Raw {
            bytes,
            unsafe_flag: _,
        }) => {
            let vendor = u16::from_be_bytes([bytes[0], bytes[1]]);
            let command = u16::from_be_bytes([bytes[2], bytes[3]]);
            let frame = Frame::new(vendor, command, bytes[4..].to_vec());
            let reply = session.request(frame)?;
            println!("{reply}");
        }
        Some(Command::Devices | Command::Doctor) => unreachable!("handled above"),
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn open_session(
    address: [u8; 6],
    channels: Option<&[u8]>,
) -> Result<Session, Box<dyn std::error::Error>> {
    match channels {
        Some(channels) => Ok(Session::open(address, channels, m4::DEFAULT_TIMEOUT)?),
        None => Ok(Session::open_resolved(address, m4::DEFAULT_TIMEOUT)?),
    }
}

#[cfg(not(target_os = "linux"))]
fn open_session(
    _address: [u8; 6],
    _channels: Option<&[u8]>,
) -> Result<Session, Box<dyn std::error::Error>> {
    Err("no GAIA transport on this platform yet (Linux only for now)".into())
}

fn get(session: &mut Session, property: &str) -> Result<String, Box<dyn std::error::Error>> {
    Ok(session.property(property)?)
}

fn validate_raw(bytes: &[u8], unsafe_flag: bool) -> Result<(), Box<dyn std::error::Error>> {
    if bytes.len() < 4 {
        return Err("need at least vendor and command: <vv vv> <cc cc> [payload]".into());
    }
    let vendor = u16::from_be_bytes([bytes[0], bytes[1]]);
    let command = u16::from_be_bytes([bytes[2], bytes[3]]);
    if blocked(vendor, command) {
        return Err(format!(
            "{vendor:#06x}:{command:#06x} is on the deny list (factory reset, DFU, pairing wipe, debug)"
        )
        .into());
    }
    if !unsafe_flag {
        return Err("raw frames are diagnostic only; re-run with --unsafe".into());
    }
    Ok(())
}

fn blocked(vendor: u16, command: u16) -> bool {
    match vendor {
        gaia::SENNHEISER => {
            matches!(command, 0x0040 | 0x1406)
                || (0x0200..=0x0203).contains(&command)
                || (0x0300..=0x0303).contains(&command)
        }
        gaia::QUALCOMM => {
            matches!(command, 0x0009 | 0x000a | 0x0803 | 0x0804)
                || (0x0c00..=0x0c02).contains(&command)
        }
        _ => false,
    }
}

fn doctor() -> Result<(), Box<dyn std::error::Error>> {
    let mut issues = 0;
    let mut check = |ok: bool, label: &str, detail: String| {
        let mark = if ok { "ok  " } else { "FAIL" };
        println!("  {mark} {label:<26} {detail}");
        if !ok {
            issues += 1;
        }
    };

    println!("bluetooth");
    let mut address = None;
    match peers() {
        Ok(devices) => {
            match devices
                .iter()
                .find(|device| device.name.to_uppercase().contains("MOMENTUM 4"))
            {
                Some(device) => {
                    address = m4::parse_address(&device.address).ok();
                    check(
                        device.connected,
                        "headset",
                        format!(
                            "{} {} ({})",
                            device.address,
                            if device.connected {
                                "connected"
                            } else {
                                "disconnected"
                            },
                            device.adapter
                        ),
                    );
                }
                None => check(false, "headset", "not paired with this adapter".to_owned()),
            }
        }
        Err(error) => check(false, "BlueZ D-Bus", error.to_string()),
    }
    match audio::autoswitch_headset_profile() {
        Ok(Some(value)) => check(true, "autoswitch to HFP", value.to_string()),
        Ok(None) => check(true, "autoswitch to HFP", "unset".to_owned()),
        Err(error) => check(false, "autoswitch to HFP", error.to_string()),
    }

    println!("audio");
    match audio::headset() {
        Ok(Some(card)) => {
            let codec = card.codec().unwrap_or("?");
            let rate = card
                .sink
                .as_ref()
                .and_then(|node| node.sample_rate_hz)
                .map(|hz| format!("{hz} Hz"))
                .unwrap_or_else(|| "idle".to_owned());
            check(
                true,
                "card",
                format!("{} {} {codec} {rate}", card.active_profile, card.name),
            );
            if !card.is_a2dp() && !card.is_headset() {
                check(
                    false,
                    "profile",
                    format!("unexpected profile {}", card.active_profile),
                );
            }
        }
        Ok(None) => check(false, "card", "no MOMENTUM 4 card in PipeWire".to_owned()),
        Err(error) => check(false, "PipeWire", error.to_string()),
    }

    println!("gaia");
    match address {
        None => check(false, "session", "no address to connect to".to_owned()),
        Some(address) => match open_session(address, None) {
            Ok(mut session) => {
                let channel = session.channel;
                let firmware = session
                    .firmware()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|error| error.to_string());
                let battery = session
                    .battery()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|error| error.to_string());
                check(
                    true,
                    "session",
                    format!("rfcomm channel {channel}, firmware {firmware}, battery {battery}%"),
                );
                let model = session.model().unwrap_or_else(|error| error.to_string());
                check(true, "identity", model);
            }
            Err(error) => {
                let message = error.to_string();
                let hint = if message.to_lowercase().contains("busy") {
                    format!("{message} (m4d or the TUI already holds the link; use `m4 status`)")
                } else {
                    message
                };
                check(false, "session", hint)
            }
        },
    }

    println!();
    if issues == 0 {
        println!("all checks passed");
        Ok(())
    } else {
        Err(format!("{issues} check(s) failed").into())
    }
}
