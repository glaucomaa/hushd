# m4

Локальное управление Sennheiser MOMENTUM 4 через Bluetooth Classic (RFCOMM + GAIA v3).
Без облака, аккаунта и телеметрии. Проверено на прошивке 3.38.3, Arch Linux с BlueZ 5.87
и macOS 15 с IOBluetooth.

## Возможности

- ANC: adaptive, custom, off; прозрачность 0–100.
- Пять полос EQ, пресеты Sennheiser, bass boost.
- Заряд, прошивка, состояние наушников и сопряжённых устройств.
- Подключение и отключение сопряжённых хостов.
- Кодек и частота из GAIA; профиль и параметры PipeWire на Linux.
- CLI, TUI в двух режимах, Waybar и MPRIS now playing на Linux.
- Нативное приложение macOS: иконка в строке меню и окно с ANC, прозрачностью,
  EQ-пресетами, bass boost и состоянием.

Один демон держит RFCOMM-сессию; клиенты подключаются к нему через UNIX-сокет.
Это исключает конфликты `EBUSY` между CLI, TUI, Waybar, строкой меню и Dock.

## Требования

Общее: Rust 1.87+ и сопряжённые MOMENTUM 4.

- Linux: BlueZ и заголовки `libbluetooth-dev` (Debian/Ubuntu) или
  `bluez-libs-devel` (Fedora); для аудиоинтеграции PipeWire, `pactl`, `wpctl`;
  для now playing `playerctl`.
- macOS 13+: IOBluetooth из системного фреймворка, Command Line Tools для сборки.

## Сборка и установка

Linux:

```bash
cargo build --release -p m4d -p cli -p tui
./scripts/install.sh
```

macOS:

```bash
./scripts/install-macos.sh
open "$HOME/Applications/M4.app"
```

Установщики копируют бинари в `~/.local/bin` и регистрируют пользовательскую службу
`m4d` через systemd или launchd. На macOS приложение устанавливается в
`~/Applications/M4.app`. Если `~/.cargo` доступен только для чтения:

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
m4 connect                      # подключить наушники через IOBluetooth
m4 disconnect                   # отключить наушники от этого Mac
m4 audio                        # профиль, кодек, частота (только Linux)
m4 audio a2dp-sink              # вернуть A2DP после HFP на Linux
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

`m4ctl` открывает собственную RFCOMM-сессию, поэтому на время диагностики `m4d`
нужно остановить.

## macOS

```bash
open "$HOME/Applications/M4.app"
```

Клик по иконке в строке меню открывает popover с основными настройками.
«Open M4…» открывает окно с EQ и сопряжёнными устройствами. Закрытие окна не завершает
приложение. Music и Spotify отдают текущий трек через AppleScript.

Приложение запускается при входе. По умолчанию открывается только строка меню.
`M4_SHOW_WINDOW=1` показывает окно сразу, `M4_MENU_BAR_ONLY=1` скрывает иконку в Dock.
`M4_CODESIGN_IDENTITY` задаёт сертификат для подписи. Лог: `~/Library/Logs/m4d.log`.

Служба перезапускается через launchd:

```bash
launchctl kickstart -k "gui/$(id -u)/dev.m4.m4d"
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
cargo test --workspace
m4ctl doctor                    # проверка на наушниках
```

## Документы

- [research.md](research.md) — протокол и проверенные факты.
- [probe/gaia-commands.md](probe/gaia-commands.md) — карта команд.
- [docs/PLAN-MACOS.md](docs/PLAN-MACOS.md) — macOS.
