#!/usr/bin/env python3
"""Adds the custom/m4 module to an existing Waybar config (idempotent).

The install script only places binaries; Waybar still needs a module entry.
Run after `omarchy-refresh-waybar` too, because that regenerates the config.

    ./scripts/install-waybar.py [--dry-run] [--no-reload]
"""
from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys

MODULE = "custom/m4"
DEFINITION = """  "custom/m4": {
    "format": "{}",
    "exec": "m4 waybar",
    "return-type": "json",
    "restart-interval": 5,
    "tooltip": true,
    "smooth-scrolling-threshold": 1,
    "on-click": "m4 anc cycle",
    "on-click-right": "alacritty -e m4-tui",
    "on-scroll-up": "m4 transparency +5",
    "on-scroll-down": "m4 transparency -5"
  }"""
STYLE = """
#custom-m4 {
  padding: 0 8px;
  margin: 0 2px;
}

#custom-m4.offline,
#custom-m4.off {
  opacity: 0.55;
}

#custom-m4.adaptive {
  font-weight: bold;
}
"""


def strip_jsonc(text: str) -> str:
    """Best-effort JSONC -> JSON so we can validate our edit."""
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    text = re.sub(r"(?m)^\s*//.*$", "", text)
    text = re.sub(r"(?<!:)//.*$", "", text, flags=re.M)
    text = re.sub(r",(\s*[}\]])", r"\1", text)
    return text


def add_to_modules_right(text: str) -> str:
    match = re.search(r'"modules-right"\s*:\s*\[', text)
    if not match:
        raise SystemExit('config has no "modules-right" array')
    start = match.end()
    end = text.find("]", start)
    if end < 0:
        raise SystemExit("unterminated modules-right array")
    if MODULE in text[start:end]:
        return text

    bluetooth = re.search(r'"bluetooth"\s*,', text[start:end])
    if bluetooth:
        position = start + bluetooth.end()
        return text[:position] + f'\n    "{MODULE}",' + text[position:]
    return text[:start] + f'\n    "{MODULE}",' + text[start:]


def replace_definition(text: str) -> tuple[str, bool]:
    match = re.search(r'"custom/m4"\s*:', text)
    if not match:
        return text, False
    brace = text.index("{", match.end())
    depth = 0
    in_string = False
    escaped = False
    end = brace
    for index in range(brace, len(text)):
        char = text[index]
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            continue
        if char == '"':
            in_string = True
        elif char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                end = index
                break
    updated = text[: match.start()] + DEFINITION.lstrip() + text[end + 1 :]
    return updated, updated == text


def add_definition(text: str) -> str:
    replaced, unchanged = replace_definition(text)
    if replaced != text:
        return replaced
    if unchanged:
        return text
    closing = text.rstrip().rfind("}")
    if closing < 0:
        raise SystemExit("config does not look like a JSON object")
    head = text[:closing].rstrip()
    comma = "" if head.endswith(("{", ",")) else ","
    return f"{text[:closing]}{comma}\n{DEFINITION}\n{text[closing:]}"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--no-reload", action="store_true")
    parser.add_argument("--config", default=None)
    parser.add_argument("--style", default=None)
    args = parser.parse_args()

    config_dir = pathlib.Path(os.environ.get("XDG_CONFIG_HOME", pathlib.Path.home() / ".config")) / "waybar"
    if args.config:
        config = pathlib.Path(args.config)
    else:
        candidates = [config_dir / "config.jsonc", config_dir / "config"]
        config = next((path for path in candidates if path.exists()), candidates[0])
    style = pathlib.Path(args.style) if args.style else config_dir / "style.css"

    if not config.exists():
        print(f"waybar config not found at {config}; nothing to do", file=sys.stderr)
        return 0

    original = config.read_text()
    updated = add_definition(add_to_modules_right(original))

    try:
        json.loads(strip_jsonc(updated))
    except json.JSONDecodeError as error:
        print(f"refusing to write: edit would break JSON ({error})", file=sys.stderr)
        return 1

    changed = updated != original
    if args.dry_run:
        print(updated if changed else "config already contains custom/m4")
        return 0

    if changed:
        backup = config.with_suffix(config.suffix + ".bak")
        shutil.copy2(config, backup)
        config.write_text(updated)
        print(f"patched {config} (backup: {backup.name})")
    else:
        print(f"{config} already has the current {MODULE} block")

    if style.exists() and "#custom-m4" not in style.read_text():
        with style.open("a") as handle:
            handle.write(STYLE)
        print(f"appended styles to {style}")

    if not args.no_reload:
        if shutil.which("omarchy-restart-waybar"):
            subprocess.run(["omarchy-restart-waybar"], check=False)
            print("waybar restarted")
        elif subprocess.run(["pkill", "-SIGUSR2", "-x", "waybar"], check=False).returncode == 0:
            print("waybar reloaded")
        else:
            print("waybar does not seem to be running; it will pick the config up on next start")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
