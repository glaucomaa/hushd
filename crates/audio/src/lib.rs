//! PipeWire-side view of a Bluetooth headset: profile, negotiated codec,
//! sample rate. The headset's own GAIA codec property is empty while idle, so
//! this crate is the source of truth for what is actually on the air.

use std::collections::{BTreeMap, HashMap};
use std::process::Command;

use serde::{Deserialize, Serialize};

const LIST_CARDS: &str = "pactl --format=json list cards";
const LIST_SINKS: &str = "pactl --format=json list sinks";
const AUTOSWITCH_KEY: &str = "bluetooth.autoswitch-to-headset-profile";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to run {command}: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{command} failed: {stderr}")]
    Exit { command: String, stderr: String },
    #[error("unexpected output from {command}: {detail}")]
    Parse { command: String, detail: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    pub name: String,
    pub description: String,
    pub sinks: u32,
    pub sources: u32,
    pub priority: u32,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub name: String,
    pub description: String,
    pub state: String,
    pub profile: Option<String>,
    pub codec: Option<String>,
    pub sample_rate_hz: Option<u32>,
    pub channels: Option<u8>,
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Card {
    pub name: String,
    pub address: String,
    pub description: String,
    pub active_profile: String,
    pub profiles: Vec<Profile>,
    pub sink: Option<Node>,
    pub source: Option<Node>,
}

impl Card {
    pub fn is_a2dp(&self) -> bool {
        self.active_profile.starts_with("a2dp-sink")
    }

    pub fn is_headset(&self) -> bool {
        self.active_profile.starts_with("headset-head-unit")
    }

    /// `api.bluez5.codec` lives on the sink, not on the card: the card's
    /// `bluez5.profile` stays "off" after a profile switch.
    pub fn codec(&self) -> Option<&str> {
        self.sink
            .as_ref()
            .and_then(|node| node.codec.as_deref())
            .or_else(|| {
                self.profiles
                    .iter()
                    .find(|profile| profile.name == self.active_profile)
                    .and_then(|profile| codec_from_description(&profile.description))
            })
    }
}

#[derive(Debug, Deserialize)]
struct RawCard {
    name: String,
    active_profile: String,
    #[serde(default)]
    properties: HashMap<String, String>,
    #[serde(default)]
    profiles: BTreeMap<String, RawProfile>,
}

#[derive(Debug, Deserialize)]
struct RawProfile {
    #[serde(default)]
    description: String,
    #[serde(default)]
    sinks: u32,
    #[serde(default)]
    sources: u32,
    #[serde(default)]
    priority: u32,
    #[serde(default)]
    available: bool,
}

#[derive(Debug, Deserialize)]
struct RawNode {
    name: String,
    description: Option<String>,
    state: Option<String>,
    sample_specification: Option<String>,
    #[serde(default)]
    properties: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct SampleSpec {
    format: String,
    channels: u8,
    rate_hz: u32,
}

pub fn parse_cards(cards_json: &str, sinks_json: &str) -> Result<Vec<Card>, Error> {
    let raw_cards: Vec<RawCard> =
        serde_json::from_str(cards_json).map_err(|error| Error::Parse {
            command: LIST_CARDS.to_owned(),
            detail: error.to_string(),
        })?;
    let raw_sinks: Vec<RawNode> =
        serde_json::from_str(sinks_json).map_err(|error| Error::Parse {
            command: LIST_SINKS.to_owned(),
            detail: error.to_string(),
        })?;

    raw_cards
        .into_iter()
        .map(|card| convert_card(card, &raw_sinks))
        .collect()
}

pub fn cards() -> Result<Vec<Card>, Error> {
    let cards_json = command_output("pactl", &["--format=json", "list", "cards"])?;
    let sinks_json = command_output("pactl", &["--format=json", "list", "sinks"])?;
    parse_cards(&cards_json, &sinks_json)
}

pub fn headset() -> Result<Option<Card>, Error> {
    headset_for(None)
}

/// Prefers the address BlueZ reports: renaming the headset changes its
/// `device.description`, which would break a name match.
pub fn headset_for(address: Option<&str>) -> Result<Option<Card>, Error> {
    let cards = cards()?;
    if let Some(address) = address {
        if let Some(card) = cards
            .iter()
            .find(|card| card.address.eq_ignore_ascii_case(address))
        {
            return Ok(Some(card.clone()));
        }
    }
    Ok(cards
        .into_iter()
        .find(|card| card.description.to_uppercase().contains("MOMENTUM 4")))
}

pub fn set_profile(card: &str, profile: &str) -> Result<(), Error> {
    command_output("pactl", &["set-card-profile", card, profile]).map(|_| ())
}

pub fn autoswitch_headset_profile() -> Result<Option<bool>, Error> {
    let output = command_output("wpctl", &["settings", AUTOSWITCH_KEY])?;
    parse_autoswitch(&output)
}

pub fn set_autoswitch_headset_profile(enabled: bool) -> Result<(), Error> {
    let value = if enabled { "true" } else { "false" };
    command_output("wpctl", &["settings", "--save", AUTOSWITCH_KEY, value]).map(|_| ())
}

fn convert_card(raw: RawCard, sinks: &[RawNode]) -> Result<Card, Error> {
    let address = property(&raw.properties, "api.bluez5.address")
        .or_else(|| property(&raw.properties, "device.string"))
        .unwrap_or_default();
    let description = property(&raw.properties, "device.description")
        .or_else(|| property(&raw.properties, "device.alias"))
        .unwrap_or_else(|| raw.name.clone());
    let profiles = raw
        .profiles
        .into_iter()
        .map(|(name, profile)| Profile {
            name,
            description: profile.description,
            sinks: profile.sinks,
            sources: profile.sources,
            priority: profile.priority,
            available: profile.available,
        })
        .collect();
    let sink = find_sink(&address, sinks).map(convert_node).transpose()?;

    Ok(Card {
        name: raw.name,
        address,
        description,
        active_profile: raw.active_profile,
        profiles,
        sink,
        source: None,
    })
}

fn find_sink<'a>(address: &str, sinks: &'a [RawNode]) -> Option<&'a RawNode> {
    sinks
        .iter()
        .filter(|sink| {
            property(&sink.properties, "api.bluez5.address").as_deref() == Some(address)
                || property(&sink.properties, "device.string").as_deref() == Some(address)
        })
        .max_by_key(|sink| (sink.state.as_deref() == Some("RUNNING"), &sink.name))
}

fn convert_node(raw: &RawNode) -> Result<Node, Error> {
    let sample = raw
        .sample_specification
        .as_deref()
        .map(parse_sample_specification)
        .transpose()?;
    Ok(Node {
        name: raw.name.clone(),
        description: raw.description.clone().unwrap_or_else(|| raw.name.clone()),
        state: raw.state.clone().unwrap_or_else(|| "UNKNOWN".to_owned()),
        profile: property(&raw.properties, "api.bluez5.profile"),
        codec: property(&raw.properties, "api.bluez5.codec"),
        sample_rate_hz: sample.as_ref().map(|sample| sample.rate_hz),
        channels: sample.as_ref().map(|sample| sample.channels),
        format: sample.map(|sample| sample.format),
    })
}

fn property(properties: &HashMap<String, String>, key: &str) -> Option<String> {
    properties
        .get(key)
        .cloned()
        .filter(|value| !value.is_empty())
}

fn parse_sample_specification(value: &str) -> Result<SampleSpec, Error> {
    let mut parts = value.split_whitespace();
    let format = parts.next();
    let channels = parts.next();
    let rate = parts.next();
    let (Some(format), Some(channels), Some(rate), None) = (format, channels, rate, parts.next())
    else {
        return Err(parse_error(
            "sample_specification",
            format!("expected '<format> <channels> <rate>Hz', got {value:?}"),
        ));
    };
    let channel_count = channels.strip_suffix("ch").ok_or_else(|| {
        parse_error(
            "sample_specification",
            format!("missing ch suffix in {value:?}"),
        )
    })?;
    let channels = channel_count.parse::<u8>().map_err(|error| {
        parse_error(
            "sample_specification",
            format!("invalid channel count in {value:?}: {error}"),
        )
    })?;
    let digits = rate.strip_suffix("Hz").ok_or_else(|| {
        parse_error(
            "sample_specification",
            format!("missing Hz suffix in {value:?}"),
        )
    })?;
    let rate_hz = digits.parse::<u32>().map_err(|error| {
        parse_error(
            "sample_specification",
            format!("invalid sample rate in {value:?}: {error}"),
        )
    })?;
    Ok(SampleSpec {
        format: format.to_owned(),
        channels,
        rate_hz,
    })
}

fn codec_from_description(description: &str) -> Option<&str> {
    let codec = description.split("codec ").nth(1)?.trim_end_matches(')');
    match codec {
        "SBC" => Some("sbc"),
        "SBC-XQ" => Some("sbc_xq"),
        "AAC" => Some("aac"),
        "aptX" => Some("aptx"),
        "aptX HD" => Some("aptx_hd"),
        "CVSD" => Some("cvsd"),
        "MSBC" => Some("msbc"),
        "" => None,
        other => Some(other),
    }
}

fn parse_autoswitch(output: &str) -> Result<Option<bool>, Error> {
    let Some(value) = output.trim().strip_prefix("Value:").map(str::trim) else {
        return Ok(None);
    };
    match value {
        "true" => Ok(Some(true)),
        "false" => Ok(Some(false)),
        "" | "null" => Ok(None),
        _ => Err(parse_error(
            "wpctl settings",
            format!("unknown boolean value {value:?}"),
        )),
    }
}

fn command_output(program: &str, args: &[&str]) -> Result<String, Error> {
    let command = std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|source| Error::Spawn {
            command: command.clone(),
            source,
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(Error::Exit { command, stderr });
    }
    String::from_utf8(output.stdout).map_err(|error| Error::Parse {
        command,
        detail: format!("output is not UTF-8: {error}"),
    })
}

fn parse_error(command: &str, detail: String) -> Error {
    Error::Parse {
        command: command.to_owned(),
        detail,
    }
}
