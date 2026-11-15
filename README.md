# m4

Локальное управление Sennheiser MOMENTUM 4 через Bluetooth Classic (RFCOMM + GAIA v3).
Без облака, аккаунта и телеметрии. Проверено на прошивке 3.38.3, Arch Linux, BlueZ 5.87,
PipeWire 1.6.9.

## Возможности

- ANC: adaptive, custom, off; прозрачность 0–100.
- Пять полос EQ, пресеты Sennheiser, bass boost.
- Заряд, прошивка, состояние наушников и сопряжённых устройств.
- Подключение и отключение сопряжённых хостов.
- Кодек, профиль и частота из PipeWire.
- CLI, TUI в двух режимах, Waybar и MPRIS now playing.

Один демон держит RFCOMM-сессию; клиенты подключаются к нему через UNIX-сокет.
Это исключает конфликты `EBUSY` между CLI, TUI и Waybar.

## Требования

- Linux, BlueZ, Rust 1.87+.
- Заголовки BlueZ: `libbluetooth-dev` (Debian/Ubuntu) или `bluez-libs-devel` (Fedora).
- Для аудиоинтеграции: PipeWire, `pactl`, `wpctl`.
- Для now playing: `playerctl`.

## Сборка и установка

```bash
cargo build --release -p m4d -p cli -p tui
./scripts/install.sh
```

Установщик копирует бинари в `~/.local/bin`, устанавливает пользовательский systemd unit
и перезапускает `m4d`. Если `~/.cargo` доступен только для чтения:

```bash
CARGO_HOME=$PWD/.cargo-home cargo build --release -p m4d -p cli -p tui
```

## Команды

```bash
m4 status                       # состояние через демон
m4 watch                        # поток изменений
m4 anc adaptive|custom|off|cycle
m4 transparency 70              # также +5 / -5
m4 eq 0 -2.5                    # полоса 0..4, ±6 дБ
m4 preset list|rock|pop|dance|hip-hop|classical|movie|jazz|neutral
m4 bass-boost on|off
m4 touch-lock on|off
m4 get battery
m4 audio                        # профиль, кодек, частота
m4 audio a2dp-sink              # вернуть A2DP после HFP
m4 doctor                       # диагностика через демон

m4-tui                          # приборная панель
m4-tui --minimal                # компактный вид, клавиша m переключает режимы
```

Прямой доступ без демона:

```bash
m4ctl info
m4ctl doctor
m4ctl devices
m4ctl --channel 15 get battery  # явный канал вместо SDP
```

## Waybar

```bash
./scripts/install-waybar.py
```

Минимальная конфигурация модуля:

```jsonc
"custom/m4": {
  "exec": "m4 waybar",
  "return-type": "json",
  "restart-interval": 5,
  "on-click": "m4 anc cycle",
  "on-click-right": "alacritty -e m4-tui",
  "on-scroll-up": "m4 transparency +5",
  "on-scroll-down": "m4 transparency -5"
}
```

`m4 waybar` подписывается на события и печатает JSON только при изменении состояния.

## Настройка

- `M4D_SOCKET=/path/m4d.sock` — путь сокета для демона и клиентов.
- `M4_AUDIO_INTERVAL=5` — интервал PipeWire в секундах.
- `M4_TRACK_INTERVAL=1` — интервал MPRIS в секундах.

Дробные интервалы разрешены; по умолчанию оба равны 2 секундам.
RFCOMM-каналы определяются через SDP; fallback — каналы 1 и 15.

## Безопасность

- Обычные команды проверяются по типу и диапазону до отправки.
- `m4ctl raw` требует `--unsafe`.
- Даже с `--unsafe` заблокированы factory reset (`0x0040`), очистка сопряжений (`0x1406`),
  `Upgrade_*`, `Debug_*` и `Core_DataTransfer*`.
- Сокет демона доступен только текущему пользователю (`0600`).
- Лицензия: [MIT](LICENSE).

## Проверка

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace          # 81 тест
m4ctl doctor                    # проверка на наушниках
```

## Документы

- [research.md](research.md) — протокол и проверенные факты.
- [probe/gaia-commands.md](probe/gaia-commands.md) — карта команд.
- [docs/PLAN-MACOS.md](docs/PLAN-MACOS.md) — план поддержки macOS.
