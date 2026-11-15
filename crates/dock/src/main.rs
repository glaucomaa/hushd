#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use ipc::Status;

fn anc_selection(info: &m4::Info) -> usize {
    if info.anc_status == Some(0) {
        2
    } else if info
        .anc
        .as_deref()
        .is_some_and(|anc| anc.contains("adaptive=1"))
    {
        0
    } else {
        1
    }
}

fn noise_mode(info: &m4::Info) -> String {
    match anc_selection(info) {
        2 => "Off".to_owned(),
        0 => "Adaptive".to_owned(),
        _ => match info.transparency {
            Some(level) => format!("Custom {level}%"),
            None => "Custom".to_owned(),
        },
    }
}

fn charging_text(charging: Option<&str>) -> String {
    match charging {
        Some("charging") => "Charging".to_owned(),
        Some("discharging") => "Discharging".to_owned(),
        Some(other) if !other.trim().is_empty() => other.to_owned(),
        _ => "Battery".to_owned(),
    }
}

fn codec_name(codec: &str) -> &str {
    match codec {
        "aptx_hd" => "aptX HD",
        "aptx" => "aptX",
        "sbc" => "SBC",
        "sbc_xq" => "SBC-XQ",
        "aac" => "AAC",
        "ldac" => "LDAC",
        "cvsd" => "CVSD",
        "msbc" => "mSBC",
        other => other,
    }
}

fn sample_rate_label(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{} kHz", rate / 1000)
    } else {
        format!("{:.1} kHz", f64::from(rate) / 1000.0)
    }
}

fn audio_summary(info: &m4::Info) -> String {
    let codec = info.codec.as_deref().unwrap_or("");
    let rate = info.sample_rate_hz.unwrap_or(0);
    if codec.is_empty() || codec == "none" || rate == 0 {
        "No audio stream".to_owned()
    } else if codec.starts_with("unknown") {
        // Unknown codecs still indicate an active stream.
        format!("Audio stream · {}", sample_rate_label(rate))
    } else {
        format!("{} · {}", codec_name(codec), sample_rate_label(rate))
    }
}

fn preset_selection(info: &m4::Info) -> Option<usize> {
    if info
        .audio_mode
        .as_deref()
        .is_some_and(|mode| mode.starts_with("off"))
    {
        return Some(m4::PRESETS.len());
    }
    m4::preset_for_gains(&info.eq_gains_db)
        .and_then(|name| m4::PRESETS.iter().position(|preset| preset.name == name))
}

fn menu_bar_title(status: &Status) -> String {
    if !status.connected {
        return "M4 off".to_owned();
    }
    let battery = status
        .info
        .battery_percent
        .map(|percent| format!("{percent}%"))
        .unwrap_or_else(|| "—".to_owned());
    let mode = match anc_selection(&status.info) {
        2 => "Off".to_owned(),
        0 => "ANC".to_owned(),
        _ => match status.info.transparency {
            Some(level) => format!("C {level}%"),
            None => "Custom".to_owned(),
        },
    };
    format!("{battery} · {mode}")
}

fn menu_tooltip(status: &Status) -> String {
    if !status.connected {
        return format!(
            "MOMENTUM 4: {}",
            status.error.as_deref().unwrap_or("not connected")
        );
    }
    let battery = status
        .info
        .battery_percent
        .map(|percent| format!("{percent}%"))
        .unwrap_or_else(|| "?".to_owned());
    let codec = status.info.codec.as_deref().unwrap_or("codec unavailable");
    let rate = status
        .info
        .sample_rate_hz
        .map(|rate| format!("{rate} Hz"))
        .unwrap_or_else(|| "idle".to_owned());
    format!(
        "MOMENTUM 4\nbattery: {battery} ({})\nnoise: {}\ncodec: {codec} · {rate}\nfirmware: {}",
        status.info.charging.as_deref().unwrap_or("unknown"),
        noise_mode(&status.info),
        status.info.firmware.as_deref().unwrap_or("?")
    )
}

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
fn main() {
    macos::main();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("m4-dock requires macOS");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected(battery: u8) -> Status {
        let mut status = Status {
            connected: true,
            ..Status::default()
        };
        status.info.battery_percent = Some(battery);
        status.info.anc = Some("adaptive=0 anti-wind=0 comfort=0".to_owned());
        status.info.anc_status = Some(1);
        status.info.transparency = Some(0);
        status
    }

    #[test]
    fn menu_bar_title_tracks_noise_control() {
        let mut status = connected(90);
        status.info.anc = Some("adaptive=1 anti-wind=0 comfort=0".to_owned());
        assert_eq!(menu_bar_title(&status), "90% · ANC");

        status.info.anc_status = Some(0);
        assert_eq!(menu_bar_title(&status), "90% · Off");

        status.info.anc_status = Some(1);
        status.info.anc = Some("adaptive=0 anti-wind=0 comfort=0".to_owned());
        status.info.transparency = Some(30);
        assert_eq!(menu_bar_title(&status), "90% · C 30%");

        status.connected = false;
        status.error = Some("headset is not connected".to_owned());
        assert_eq!(menu_bar_title(&status), "M4 off");
        assert_eq!(
            menu_tooltip(&status),
            "MOMENTUM 4: headset is not connected"
        );
    }

    #[test]
    fn anc_selection_maps_protocol_flags_to_menu_order() {
        let mut info = m4::Info::default();
        assert_eq!(anc_selection(&info), 1);
        info.anc = Some("adaptive=1 anti-wind=0 comfort=0".to_owned());
        assert_eq!(anc_selection(&info), 0);
        info.anc_status = Some(0);
        assert_eq!(anc_selection(&info), 2);
    }

    #[test]
    fn preset_selection_recognizes_curves_and_headset_tuning() {
        let mut info = m4::Info::default();
        assert_eq!(preset_selection(&info), None);

        info.eq_gains_db = m4::PRESETS[2].gains_db.to_vec();
        assert_eq!(preset_selection(&info), Some(2));

        info.audio_mode = Some("off (headset tuning)".to_owned());
        assert_eq!(preset_selection(&info), Some(m4::PRESETS.len()));
    }

    #[test]
    fn charging_and_audio_labels_are_human_readable() {
        assert_eq!(charging_text(Some("discharging")), "Discharging");
        assert_eq!(charging_text(Some("charging")), "Charging");
        assert_eq!(charging_text(None), "Battery");

        let mut info = m4::Info::default();
        assert_eq!(audio_summary(&info), "No audio stream");
        info.codec = Some("none".to_owned());
        info.sample_rate_hz = Some(0);
        assert_eq!(audio_summary(&info), "No audio stream");
        info.codec = Some("aptx_hd".to_owned());
        info.sample_rate_hz = Some(48_000);
        assert_eq!(audio_summary(&info), "aptX HD · 48 kHz");
        info.sample_rate_hz = Some(44_100);
        assert_eq!(audio_summary(&info), "aptX HD · 44.1 kHz");
        info.codec = Some("unknown(240)".to_owned());
        info.sample_rate_hz = Some(16_000);
        assert_eq!(audio_summary(&info), "Audio stream · 16 kHz");
    }
}
