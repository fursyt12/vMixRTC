# Сборка под платформы

Проект — обычное Tauri v2-приложение (`crates/vmixrtc`) плюс портируемое ядро на Rust.
Все команды выполняются **из корня репозитория** (весь код лежит там, папки `rust/` больше нет).
Ниже: что уже проверено в этом окружении, что требует платформенного тулчейна и точные команды.

## 1. Проверка переносимости

```bash
./check-targets.sh                # Linux + ARM + Windows + Apple + Android
./check-targets.sh aarch64-linux-android   # только одна цель
```

Скрипт делает `cargo check` (без линковки и SDK) по портируемым крейтам. Результат на момент
написания:

| крейт | linux x86_64 | linux ARM64 | Windows x86_64 | macOS ARM64 | iOS ARM64 | Android ARM64 |
|---|---|---|---|---|---|
| `vmix-xml`, `vmix-config`, `vmix-functions`, `vmix-script` | ✅ | ✅ | ✅ (gnu/msvc) | ✅ | ✅ | ✅ |
| `vmix-ndi` (FFI к NDI через `libloading`) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| `vmix-midi` (`midir`, только десктоп) | ✅ | ✅ | ✅ | ✅ | ✅ (заглушка) | ✅ (заглушка) |
| `vmix-streamdeck` (HID, только десктоп) | ✅ | ✅ | ✅ | ✅ | ✅ (заглушка) | ✅ (заглушка) |
| `vmix-api`, `vmix-providers`, `vmixrtc-cli` | ✅ | ⚠️ нужен кросс-cc | ✅ gnu (собран `.exe`), ⚠️ msvc нужен `cl.exe` | ⚠️ нужен Xcode | ⚠️ нужен Xcode | ⚠️ нужен NDK |
| `vmixrtc` (Tauri) | ✅ | ⚠️ SDK платформы | ✅ gnu (собран `.exe`), ⚠️ msvc нужен `cl.exe` | ⚠️ Xcode | ⚠️ Xcode | ⚠️ NDK + SDK |

**Почему ⚠️, а не ошибка кода:** единственная причина — `ring` (TLS для `ureq`), которому нужен
C-компилятор целевой платформы. Это стандартное требование кросс-сборки: `gcc-aarch64-linux-gnu`
для ARM, Xcode для Apple, NDK для Android. Ядро, скрипты, каталог функций, NDI/MIDI/Stream Deck и
CLI собираются под все цели без платформенных зависимостей.

В песочнице пришлось поставить цели в локальный `RUSTUP_HOME` (домашний `~/.rustup` только для
чтения):

```bash
mkdir -p .rustup/toolchains
cp -a ~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu .rustup/toolchains/
RUSTUP_HOME=$PWD/.rustup rustup default stable-x86_64-unknown-linux-gnu
RUSTUP_HOME=$PWD/.rustup rustup target add aarch64-apple-darwin aarch64-apple-ios \
    aarch64-unknown-linux-gnu aarch64-linux-android
```

### Windows: что уже проверено здесь

В этом окружении есть `mingw-w64` и `wine`, поэтому Windows-сборка не только проверена типами,
но и **собрана**:

```bash
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu -p vmixrtc-cli   # → vmixrtc-cli.exe (3,6 МБ, PE32+)
cargo build --release --target x86_64-pc-windows-gnu -p vmixrtc       # → vmixrtc.exe (10,9 МБ, PE32+ GUI)
# проверка CLI-бинарника под wine:
WINEPREFIX=$PWD/.wine wine target/x86_64-pc-windows-gnu/release/vmixrtc-cli.exe verify examples
#   файлов: 6, виджетов: 57, команд: 45, проблем: 0
```

* Обе цели (`x86_64-pc-windows-gnu` и `x86_64-pc-windows-msvc`) проходят `cargo check`; MSVC
  дополнительно требует `cl.exe` (Visual Studio Build Tools) — в песочнице его нет.
* `.exe` собраны mingw-тулчейном и запускаются (GUI стартует, для отрисовки нужен WebView2).
* **Инсталляторы (.msi/.exe-NSIS) собираются на Windows** командой `cargo tauri build` — это и
  делает CI-пайплайн (см. ниже): локально WiX/NSIS нет.

## 2. Десктоп

| платформа | подготовка | сборка |
|---|---|---|
| **Linux** | `webkit2gtk-4.1`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `libasound2-dev` (MIDI), `libudev-dev` (HID), `pkg-config`, `build-essential` | `cd crates/vmixrtc && cargo tauri build` |
| **macOS** | Xcode Command Line Tools; для подписи — сертификат Developer ID | **universal** (Apple Silicon + Intel): `rustup target add aarch64-apple-darwin x86_64-apple-darwin` затем `cd crates/vmixrtc && cargo tauri build --target universal-apple-darwin` → один `.dmg` под оба процессора |
| **Windows** | MSVC Build Tools + WebView2 Runtime (или mingw-w64 для `gnu`-цели) | `cd crates/vmixrtc && cargo tauri build` → `.msi`/`.exe` |

Профиль release в корневом `Cargo.toml` настроен на компактные и быстрые артефакты:
`codegen-units = 1`, `lto = "thin"`, `opt-level = 3`, `strip = true`.

Для запуска из исходников: `cargo run -p vmixrtc -- <файл.vmc> [индекс] [script|rows|midi|deck|schedule]`.

## 3. ARM SBC (Raspberry Pi и подобные)

```bash
rustup target add aarch64-unknown-linux-gnu
sudo apt install gcc-aarch64-linux-gnu          # кросс-компилятор для ring
mkdir -p .cargo && cat >> .cargo/config.toml <<'CFG'
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"
CFG
cargo build --release --target aarch64-unknown-linux-gnu -p vmixrtc-cli   # CLI и ядро
# UI: проще собирать на самой плате (нужны ARM-сборки webkit2gtk):
#   cd crates/vmixrtc && cargo tauri build
```

## 4. Android

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
export ANDROID_HOME=$HOME/Android/Sdk
export NDK_HOME=$ANDROID_HOME/ndk/26.1.10909125
export JAVA_HOME=/usr/lib/jvm/java-17-openjdk
cd crates/vmixrtc
cargo tauri android init      # один раз: создаёт gen/android и Gradle-проект
cargo tauri android dev       # запуск на устройстве/эмуляторе
cargo tauri android build     # APK/AAB
```

Особенности порта на Android:

* **MIDI и Stream Deck выключены** (`vmix-midi`/`vmix-streamdeck` без фич `midi`/`hid`) и честно
  сообщают об этом в интерфейсе — нативных бэкендов для Android у этих крейтов нет.
* **NDI** работает, если положить NDI SDK для Android рядом с библиотекой (FFI через `libloading`).
* **TLS** (Google Sheets, HTTPS-провайдеры) требует NDK: `ring` собирается NDK-компилятором
  автоматически, если задан `NDK_HOME`.
* Каталог настроек — `$XDG_CONFIG_HOME`/`~/.config/vmixrtc`; на Android это внутренний каталог
  приложения.

## 5. iOS

Только на macOS с Xcode (кросс-сборка с Linux невозможна даже для `check` без SDK):

```bash
rustup target add aarch64-apple-ios x86_64-apple-ios aarch64-apple-ios-sim
cd crates/vmixrtc
cargo tauri ios init          # один раз: создаёт gen/apple и Xcode-проект
cargo tauri ios dev           # на подключённом устройстве/симуляторе
cargo tauri ios build         # архив для App Store (нужна подпись)
```

Ограничения те же, что на Android (MIDI/HID-заглушки, NDI — только с SDK под iOS).

## 5a. Готовые сборки через CI

`.github/workflows/rust.yml` собирает **Windows, macOS и Linux**:

| шаг | что делает |
|---|---|
| `test` | `cargo test --workspace` и `vmixrtc-cli verify examples` на ubuntu/windows/macos |
| `bundle` | `cargo tauri build` → `.msi`/`.exe` (Windows), `.dmg`/`.app` (macOS), `.deb`/`.AppImage` (Linux) |
| `release` | по тегу `rust-v*` публикует GitHub Release с zip-архивами; alpha/beta/rc и `rust-test-*` — prerelease |

Запуск вручную: Actions → «vMixRTC» → Run workflow (можно указать версию в имени артефакта).

## 6. Что ещё нужно сделать для мобильных сборок

* `gen/android` и `gen/apple` создаются командами `tauri android init` / `tauri ios init` на
  машине с тулчейном — в этом окружении их нет (нет NDK/Xcode).
* Иконки приложений: набор `icons/32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.png`,
  `icon.ico` уже сгенерирован и прописан в `tauri.conf.json`; для iOS нужен `icon.icns`
  (`cargo tauri icon` на macOS делает весь набор).
* Разрешения Android (сеть) выдаются Automatically Tauri-плагином; при добавлении плагинов
  правится `gen/android/app/src/main/AndroidManifest.xml`.
* Проверка на реальных устройствах: MIDI-пульт, Stream Deck, приём NDI — единственные участки,
  которые нельзя проверить в песочнице.
