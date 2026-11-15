use audio::{parse_cards, Card};

const CARDS_A2DP: &str = include_str!("fixtures/cards-a2dp.json");
const SINKS_A2DP: &str = include_str!("fixtures/sinks-a2dp.json");
const CARD_HFP: &str = include_str!("fixtures/card-hfp.synthetic.json");

fn momentum(cards: &[Card]) -> &Card {
    cards
        .iter()
        .find(|card| card.description == "MOMENTUM 4")
        .expect("fixture must contain MOMENTUM 4")
}

#[test]
fn a2dp_fixture_reads_card_profile_and_address() {
    let cards = parse_cards(CARDS_A2DP, SINKS_A2DP).expect("valid fixtures");
    assert_eq!(cards.len(), 2);
    let card = momentum(&cards);
    assert_eq!(card.name, "bluez_card.AA_BB_CC_DD_EE_FF");
    assert_eq!(card.address, "AA:BB:CC:DD:EE:FF");
    assert_eq!(card.active_profile, "a2dp-sink");
    assert_eq!(card.profiles.len(), 8);
    assert!(card.is_a2dp());
    assert!(!card.is_headset());
}

#[test]
fn a2dp_fixture_reads_sink_codec_format_and_state() {
    let cards = parse_cards(CARDS_A2DP, SINKS_A2DP).expect("valid fixtures");
    let card = momentum(&cards);
    assert_eq!(card.codec(), Some("aptx_hd"));
    let sink = card.sink.as_ref().expect("A2DP sink");
    assert_eq!(sink.name, "bluez_output.AA_BB_CC_DD_EE_FF.1");
    assert_eq!(sink.sample_rate_hz, Some(48_000));
    assert_eq!(sink.format.as_deref(), Some("s24le"));
    assert_eq!(sink.channels, Some(2));
    assert_eq!(sink.state, "RUNNING");
    assert_eq!(sink.profile.as_deref(), Some("a2dp-sink"));
}

#[test]
fn sink_codec_wins_over_profile_description() {
    let cards_json = CARDS_A2DP.replace("codec aptX HD", "codec AAC");
    let cards = parse_cards(&cards_json, SINKS_A2DP).expect("valid fixtures");
    assert_eq!(momentum(&cards).codec(), Some("aptx_hd"));
}

#[test]
fn hfp_fixture_uses_profile_codec_without_sink() {
    let cards = parse_cards(CARD_HFP, "[]").expect("valid HFP card");
    let card = momentum(&cards);
    assert!(card.is_headset());
    assert!(!card.is_a2dp());
    assert_eq!(card.codec(), Some("msbc"));
    assert!(card.sink.is_none());
}

#[test]
fn cvsd_profile_is_normalized() {
    let card_json = CARD_HFP.replace(
        "\"active_profile\": \"headset-head-unit\"",
        "\"active_profile\": \"headset-head-unit-cvsd\"",
    );
    let cards = parse_cards(&card_json, "[]").expect("valid CVSD card");
    let card = momentum(&cards);
    assert_eq!(card.active_profile, "headset-head-unit-cvsd");
    assert_eq!(card.codec(), Some("cvsd"));
}

#[test]
fn parses_mono_16khz_sample_specification() {
    let (card_json, sink_json) = one_card_with_sink("s16le 1ch 16000Hz");
    let cards = parse_cards(&card_json, &sink_json).expect("valid sample specification");
    let sink = cards[0].sink.as_ref().expect("sink");
    assert_eq!(sink.format.as_deref(), Some("s16le"));
    assert_eq!(sink.channels, Some(1));
    assert_eq!(sink.sample_rate_hz, Some(16_000));
}

#[test]
fn rejects_invalid_sample_specification() {
    let (card_json, sink_json) = one_card_with_sink("not a sample spec");
    assert!(parse_cards(&card_json, &sink_json).is_err());
}

#[test]
fn empty_arrays_produce_no_cards() {
    let cards = parse_cards("[]", "[]").expect("empty arrays are valid");
    assert!(cards.is_empty());
}

#[test]
fn missing_sink_falls_back_to_profile_codec() {
    let cards = parse_cards(CARDS_A2DP, "[]").expect("cards without sinks are valid");
    let card = momentum(&cards);
    assert!(card.sink.is_none());
    assert_eq!(card.codec(), Some("aptx_hd"));
}

#[test]
fn malformed_json_returns_error() {
    assert!(parse_cards("not json", "[]").is_err());
    assert!(parse_cards("[]", "not json").is_err());
}

#[test]
fn missing_card_fields_return_error_instead_of_panicking() {
    assert!(parse_cards("[{}]", "[]").is_err());
}

fn one_card_with_sink(sample_specification: &str) -> (String, String) {
    let card = r#"[{
        "name": "bluez_card.AA_BB_CC_DD_EE_FF",
        "active_profile": "headset-head-unit",
        "properties": {
            "device.description": "MOMENTUM 4",
            "api.bluez5.address": "AA:BB:CC:DD:EE:FF"
        },
        "profiles": {}
    }]"#;
    let sink = format!(
        r#"[{{
            "name": "bluez_output.AA_BB_CC_DD_EE_FF.1",
            "description": "MOMENTUM 4",
            "state": "RUNNING",
            "sample_specification": "{sample_specification}",
            "properties": {{"api.bluez5.address": "AA:BB:CC:DD:EE:FF"}}
        }}]"#
    );
    (card.to_owned(), sink)
}
