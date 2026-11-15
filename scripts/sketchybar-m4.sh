#!/usr/bin/env bash
# SketchyBar item for MOMENTUM 4. setup:
#   sketchybar --add item m4 right \
#              --set m4 update_freq=5 script="$HOME/.local/bin/sketchybar-m4" \
#              --set m4 label="󰋋 ..."
set -u

status=$(m4 status --json 2>/dev/null)
if [[ -z $status ]]; then
    sketchybar --set m4 label="󰋋 off" drawing=on
    exit 0
fi

connected=$(jq -r '.connected // false' <<<"$status")
if [[ $connected != true ]]; then
    sketchybar --set m4 label="󰋋 off"
    exit 0
fi

battery=$(jq -r '.info.battery_percent // "?"' <<<"$status")
anc=$(jq -r '.info.anc // ""' <<<"$status")
level=$(jq -r '.info.transparency // 0' <<<"$status")
status_anc=$(jq -r '.info.anc_status // 1' <<<"$status")
codec=$(jq -r '.audio.codec // .info.codec // "?"' <<<"$status")

if [[ $status_anc == 0 ]]; then
    mode="off"
elif [[ $anc == *"adaptive=1"* ]]; then
    mode="anc"
else
    mode="custom $level"
fi

sketchybar --set m4 label="󰂂 ${battery}% · ${mode}" \
    --set m4 background.color=0x00000000 \
    --set m4 label.drawing=on
