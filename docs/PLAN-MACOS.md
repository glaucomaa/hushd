# macOS: план поддержки

Сейчас транспорт работает только на Linux. TUI, CLI и IPC платформенно-нейтральны.
Для macOS нужен отдельный RFCOMM-бэкенд.

## Подход

- Использовать `IOBluetoothRFCOMMChannel` через `objc2-io-bluetooth`.
- Устройства получать из `IOBluetoothDevice.pairedDevices()`.
- GAIA-канал искать по UUID через `IOBluetoothSDPServiceRecord`.
- Асинхронный канал запускать в потоке с `CFRunLoop`.
- Сохранить существующий трейт `m4::Transport` (`send`/`recv` с таймаутом).
- Разрешение Bluetooth (TCC) при первом запуске получать интерактивно.

Справка Apple: [IOBluetoothRFCOMMChannel](https://developer.apple.com/documentation/iobluetooth/iobluetoothrfcommchannel).

## Уже готово

- Тонкий трейт транспорта и тесты на моках.
- Linux RFCOMM + SDP с fallback на каналы 1 и 15.
- Платформенные границы в `m4d` и `m4ctl`.
- TUI на Ratatui.
- [SketchyBar-модуль](../scripts/sketchybar-m4.sh).
- [Шаблон launchd](../packaging/m4d.launchd.plist).

## Фазы

1. `rfcomm-macos`: SDP, async RFCOMM, поток с `CFRunLoop`, таймауты.
2. Выбор бэкенда через `cfg(target_os)` в `m4d` и `m4ctl`.
3. Установщик launchd и первый интерактивный запуск для TCC.
4. Аудио: сначала GAIA codec/sample rate; профили A2DP/HFP не переключать.
5. Проверка TUI и SketchyBar на реальном Mac.
6. Сборка `aarch64-apple-darwin`, universal binary, Homebrew, подпись.

## Риски

- `IOBluetooth` — deprecated API.
- Async-канал и run loop могут зависать; обязательны таймауты и watchdog.
- Доступ Bluetooth для launchd выдаётся бинарю демона, а не терминалу.
- Поведение двух GAIA-каналов на macOS нужно проверить на устройстве.

## Минимальная проверка на Mac

- `uname -m`, `sw_vers`.
- Наушники сопряжены в System Settings → Bluetooth.
- `pairedDevices()` видит MOMENTUM 4.
- SDP возвращает RFCOMM-канал.
- Запрос `FF 03 00 00 04 95 06 03` возвращает заряд.
