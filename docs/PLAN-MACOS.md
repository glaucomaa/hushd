# macOS

Поддержка macOS 13+. Проверено на macOS 15, прошивка 3.38.3.

## Состав

- `crates/rfcomm/src/macos`: IOBluetooth, SDP и RFCOMM.
- `crates/dock`: AppKit, строка меню, popover и основное окно.
- `crates/mpris/src/macos.rs`: Music и Spotify через AppleScript.
- GAIA, IPC, CLI и TUI общие с Linux.

Сокет: `~/Library/Application Support/m4/m4d.sock`.
Кодек и частота читаются через GAIA. Аудиопрофили управляются macOS.

## Приложение

- Строка меню: заряд и режим ANC.
- Popover: ANC, прозрачность, звук, трек и Connect/Disconnect.
- Окно: EQ, сопряжённые устройства и технические данные.
- Ручной Disconnect сохраняется до явного Connect.
- После внешнего разрыва reconnect ждёт 3 секунды.
- `M4_SHOW_WINDOW=1`: открыть окно при запуске.
- `M4_MENU_BAR_ONLY=1`: скрыть иконку в Dock.
- `M4_NO_AUTOSTART=1`: не регистрировать запуск при входе.

## Установка

```bash
./scripts/install-macos.sh
open "$HOME/Applications/M4.app"
```

Установка не требует администратора. `m4d` находится внутри `M4.app` и запускается через
launchd. Bundle подписывается; сертификат задаётся через `M4_CODESIGN_IDENTITY`.
Лог `~/Library/Logs/m4d.log` ротируется после 1 MiB.

Проверка:

```bash
m4 status
m4 doctor
cargo test --workspace
```

IOBluetooth является deprecated API. Universal binary, notarization и переключение
аудиопрофилей не реализованы.
