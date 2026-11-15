#!/usr/bin/env bash
# Waybar custom module: prints one line of MOMENTUM 4 state as Waybar JSON.
set -u

status=$(m4 status --json 2>/dev/null)
if [[ -z $status ]]; then
    echo '{"text":"󰋋 off","tooltip":"m4d is not running","class":"offline"}'
    exit 0
fi

IFS=$'\t' read -r connected battery charging codec anc level < <(
    jq -r '[
        (.connected // false),
        (.info.battery_percent // "?"),
        (.info.charging // "?"),
        (.info.codec // "?"),
        (.info.anc // "?"),
        (.info.transparency // "?")
    ] | @tsv' <<<"$status"
)

if [[ $connected != true ]]; then
    error=$(jq -r '.error // "disconnected"' <<<"$status")
    echo "{\"text\":\"󰋋 off\",\"tooltip\":\"MOMENTUM 4: ${error}\",\"class\":\"offline\"}"
    exit 0
fi

# Mode: adaptive when the ANC pair says adaptive=1, otherwise custom/off by ANC_Status.
if [[ $anc == *"adaptive=1"* ]]; then
    mode="adaptive"
elif [[ $(jq -r '.info.anc_status // 1' <<<"$status") == 0 ]]; then
    mode="off"
else
    mode="custom ${level}"
fi

if [[ $charging == charging* ]]; then
    icon="󰂄"
elif (( battery >= 80 )); then
    icon="󰂂"
elif (( battery >= 60 )); then
    icon="󰂁"
elif (( battery >= 40 )); then
    icon="󰂀"
elif (( battery >= 20 )); then
    icon="󰁾"
else
    icon="󰁻"
fi

text="${icon} ${battery}% · ${mode}"
tooltip="MOMENTUM 4
battery: ${battery}% (${charging})
noise: ${mode}
codec: ${codec}"

jq -n --arg text "$text" --arg tooltip "$tooltip" --arg class "${mode%% *}" \
    '{text: $text, tooltip: $tooltip, class: $class}'
