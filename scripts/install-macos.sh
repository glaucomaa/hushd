#!/usr/bin/env bash
# Installs M4 into the current user's applications, binaries and LaunchAgents.
set -euo pipefail

prefix=${PREFIX:-$HOME/.local}
applications=${APPLICATIONS:-$HOME/Applications}
unit_dir=${UNIT_DIR:-$HOME/Library/LaunchAgents}
app="$applications/M4.app"
cargo=${CARGO:-cargo}
identity=${M4_CODESIGN_IDENTITY:--}

cd "$(dirname "$0")/.."

"$cargo" build --release -p m4d -p cli -p tui -p m4ctl -p dock
if [ ! -f packaging/m4.icns ]; then
    python3 scripts/make-icon.py
    iconutil -c icns target/m4.iconset -o packaging/m4.icns
fi

install -d "$prefix/bin" "$unit_dir" \
    "$app/Contents/MacOS" "$app/Contents/Resources"
install -m 755 target/release/m4d "$app/Contents/MacOS/m4d"
install -m 755 target/release/m4 "$prefix/bin/m4"
install -m 755 target/release/m4-tui "$prefix/bin/m4-tui"
install -m 755 target/release/m4ctl "$prefix/bin/m4ctl"
install -m 755 target/release/m4-dock "$app/Contents/MacOS/m4-dock"
install -m 644 packaging/m4-app.plist "$app/Contents/Info.plist"
install -m 644 packaging/m4.icns "$app/Contents/Resources/m4.icns"
ln -sf "$app/Contents/MacOS/m4d" "$prefix/bin/m4d"

codesign --force --sign "$identity" "$app/Contents/MacOS/m4d"
codesign --force --sign "$identity" "$app/Contents/MacOS/m4-dock"
codesign --force --sign "$identity" "$app"

sed \
    -e "s|__M4D__|$app/Contents/MacOS/m4d|g" \
    -e "s|__SOCKET__|$HOME/Library/Application Support/m4/m4d.sock|g" \
    packaging/m4d.launchd.plist > "$unit_dir/dev.m4.m4d.plist"

if [ "${NO_LAUNCH:-0}" != "1" ]; then
    launchctl bootout "gui/$(id -u)/dev.m4.m4d" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$unit_dir/dev.m4.m4d.plist"
    launchctl kickstart -k "gui/$(id -u)/dev.m4.m4d"
fi

cat <<MSG
installed:
  $app (m4d and Dock app, codesign identity: $identity)
  $prefix/bin/m4
  $prefix/bin/m4-tui
  $prefix/bin/m4ctl
  $prefix/bin/m4d -> $app/Contents/MacOS/m4d
  $unit_dir/dev.m4.m4d.plist

next:
  open "$app"
  $prefix/bin/m4 status

The app registers itself for launch at login through SMAppService. Allow Bluetooth
access if macOS asks. Logs rotate at 1 MiB in ~/Library/Logs/m4d.log.
For a stable TCC identity, set M4_CODESIGN_IDENTITY to your signing certificate.
MSG
