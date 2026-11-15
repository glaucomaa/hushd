#!/usr/bin/env bash
# Installs m4d, m4, m4-tui and the systemd user unit. PREFIX defaults to ~/.local.
set -euo pipefail

prefix=${PREFIX:-$HOME/.local}
unit_dir=${UNIT_DIR:-$HOME/.config/systemd/user}
cargo=${CARGO:-cargo}

cd "$(dirname "$0")/.."

"$cargo" build --release -p m4d -p cli -p tui

install -Dm755 target/release/m4d "$prefix/bin/m4d"
install -Dm755 target/release/m4 "$prefix/bin/m4"
install -Dm755 target/release/m4-tui "$prefix/bin/m4-tui"
install -Dm755 scripts/waybar-m4.sh "$prefix/bin/waybar-m4"
install -Dm644 packaging/m4d.service "$unit_dir/m4d.service"

if command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then
    systemctl --user daemon-reload
    systemctl --user enable m4d >/dev/null 2>&1 || true
    if systemctl --user restart m4d 2>/dev/null; then
        echo "m4d restarted"
    else
        echo "m4d not started; run: systemctl --user enable --now m4d"
    fi
else
    echo "systemd user session unavailable; start the daemon manually: m4d &"
fi

if [ "${NO_WAYBAR_PATCH:-0}" != "1" ] && [ -f "${XDG_CONFIG_HOME:-$HOME/.config}/waybar/config.jsonc" ]; then
    python3 "$(dirname "$0")/install-waybar.py" || true
fi

cat <<MSG
installed:
  $prefix/bin/m4d
  $prefix/bin/m4
  $prefix/bin/m4-tui
  $prefix/bin/waybar-m4
  $unit_dir/m4d.service

next:
  m4 status                    # daemon is restarted by this script
  systemctl --user status m4d  # if something looks wrong

waybar module:
  "custom/m4": { "exec": "m4 waybar", "return-type": "json", "restart-interval": 5 }
MSG
