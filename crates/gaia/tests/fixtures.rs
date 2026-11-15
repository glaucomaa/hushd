use std::collections::HashMap;

use gaia::{
    cmd, AncModes, Charging, Codec, Deframer, EqConfig, Firmware, Frame, Kind, PairedDevice,
    PhysicalState, SliderGain,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Capture {
    firmware: String,
    model_id: String,
    exchanges: Vec<Exchange>,
}

#[derive(Deserialize)]
struct Exchange {
    request: String,
    response: Option<String>,
    command: String,
    kind: String,
}

fn unhex(text: &str) -> Vec<u8> {
    text.split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).expect("hex byte"))
        .collect()
}

fn capture() -> Capture {
    serde_json::from_str(include_str!("fixtures/m4_3.38.3.json")).expect("capture parses")
}

#[test]
fn every_captured_request_round_trips_through_the_deframer() {
    let capture = capture();
    assert_eq!(capture.exchanges.len(), 41);

    for exchange in &capture.exchanges {
        let raw = unhex(&exchange.request);
        let mut deframer = Deframer::new();
        deframer.push(&raw);
        let frame = deframer.next_frame().expect("one request frame");
        assert_eq!(
            deframer.next_frame(),
            None,
            "{} leaves no tail",
            exchange.command
        );
        assert_eq!(frame.kind(), Kind::Command, "{}", exchange.command);
        assert_eq!(
            frame.encode().unwrap(),
            raw,
            "{} re-encodes",
            exchange.command
        );
        assert_eq!(
            frame.command,
            u16::from_str_radix(&exchange.command[2..], 16).unwrap()
        );
    }
}

#[test]
fn every_captured_response_matches_its_request() {
    let capture = capture();
    for exchange in &capture.exchanges {
        let base = u16::from_str_radix(&exchange.command[2..], 16).unwrap();
        let Some(response) = &exchange.response else {
            continue;
        };
        let payload = unhex(response);
        let expected = match exchange.kind.as_str() {
            "RSP" => base | 0x0100,
            "ERR" => base | 0x0180,
            other => panic!("unexpected reply kind {other}"),
        };
        let frame = Frame::sen(expected, payload);
        assert_eq!(frame.base(), base, "{}", exchange.command);
        assert_eq!(frame.kind() == Kind::Error, exchange.kind == "ERR");
    }
}

#[test]
fn decodes_the_captured_state() {
    let capture = capture();
    let mut replies: HashMap<u16, Vec<Vec<u8>>> = HashMap::new();
    for exchange in &capture.exchanges {
        let Some(payload) = exchange.response.as_ref() else {
            continue;
        };
        if exchange.kind != "RSP" {
            continue;
        }
        let base = u16::from_str_radix(&exchange.command[2..], 16).unwrap();
        replies.entry(base).or_default().push(unhex(payload));
    }

    let get =
        |command: u16| -> &[u8] { replies.get(&command).expect("captured reply")[0].as_slice() };
    let all = |command: u16| -> Vec<Vec<u8>> { replies.get(&command).cloned().unwrap_or_default() };

    assert_eq!(
        Firmware::parse(get(cmd::FW_VERSION_GET))
            .unwrap()
            .to_string(),
        "3.38.3"
    );
    assert_eq!(capture.firmware, "3.38.3");
    assert_eq!(
        String::from_utf8_lossy(get(cmd::MODEL_ID_GET)).trim_end_matches('\0'),
        capture.model_id
    );
    assert_eq!(get(cmd::BATTERY_GET), [100]);
    assert_eq!(
        Charging::parse(get(cmd::CHARGING_GET)).unwrap().label(),
        "discharging"
    );

    let anc = AncModes::parse(get(cmd::ANC_GET)).unwrap();
    assert_eq!(
        anc,
        AncModes {
            anti_wind: 0,
            comfort: 0,
            adaptive: 0
        }
    );
    assert_eq!(get(cmd::TRANSPARENCY_GET), [69]);
    assert_eq!(get(cmd::ANC_STATUS_GET), [1]);

    let eq = EqConfig::parse(get(cmd::EQ_CONFIG_GET)).unwrap();
    assert_eq!((eq.bands, eq.min_db, eq.max_db), (5, -6.0, 6.0));
    let gains: Vec<i8> = all(cmd::EQ_SLIDER_GET)
        .iter()
        .map(|payload| SliderGain::parse(payload).unwrap().0)
        .collect();
    assert_eq!(
        gains,
        vec![0, 20, 25, 15, -20],
        "Rock preset stored on the headset"
    );

    assert_eq!(Codec::from_u8(get(cmd::CODEC_GET)[0]).to_string(), "none");
    assert_eq!(get(cmd::PHYSICAL_STATE_GET), [2]);
    assert_eq!(PhysicalState::from_u8(2).to_string(), "not on head");
    assert_eq!(get(cmd::TRANSPARENCY_MODE_GET), [1]);
}

#[test]
fn decodes_the_captured_paired_list() {
    let capture = capture();
    let devices: Vec<PairedDevice> = capture
        .exchanges
        .iter()
        .filter(|exchange| exchange.command == "0x1401")
        .filter_map(|exchange| exchange.response.as_ref())
        .map(|payload| PairedDevice::parse(&unhex(payload)).unwrap())
        .collect();

    assert_eq!(devices.len(), 4);
    assert_eq!(devices[0].name, "laptop");
    assert_eq!(devices[1].index, 1);
    assert!(devices[1].connected);
    assert_eq!(devices[2].name, "windows");
    assert_eq!(devices[3].name, "phone");
}
