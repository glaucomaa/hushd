# audio

PipeWire/PipeWire-Pulse status and profile control for MOMENTUM 4. No GAIA or RFCOMM code is used here.
The negotiated codec is read from `sinks[].properties["api.bluez5.codec"]`; the card's `bluez5.profile` can remain stale (`off`).
Cards and sinks are matched by `api.bluez5.address`, with `device.string` as fallback.

```bash
cargo run -p audio -- status
cargo run -p audio -- status --json
cargo run -p audio -- profiles
cargo run -p audio -- set-profile a2dp-sink
cargo run -p audio -- set-profile headset-head-unit
cargo run -p audio -- autoswitch
cargo run -p audio -- autoswitch off
```

`autoswitch` is WirePlumber's global `bluetooth.autoswitch-to-headset-profile` setting, not a per-headset option.
