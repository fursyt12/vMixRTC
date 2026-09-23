# vMixRTC — vMix Rust Title Controller

**vMixRTC** is a cross-platform controller for [vMix](https://vmix.com) written entirely in Rust:
title/scoreboard widgets, a script engine, data providers, MIDI, Stream Deck and an NDI monitor.

It is a full rewrite of [vMixUTC](https://github.com/elgarf/vMixUTC) (the original C#/WPF
application) — same `.vmc` controller files, same widget behaviour, but a single Rust codebase with
no .NET, no WPF and no Windows-only dependencies.

> The C#/WPF original has been removed from this repository. It stays available upstream at
> [elgarf/vMixUTC](https://github.com/elgarf/vMixUTC); this fork continues as the Rust project.

## Install

**Arch Linux** — one command (our own pacman repository, no AUR account needed):

```bash
curl -fsSL https://github.com/fursyt12/vMixRTC/releases/download/repo-x86_64/vmixrtc-repo.sh | sudo bash
sudo pacman -S vmixrtc-bin
```

Prefer to look first? The same installer as a file, plus the manual variant:

```bash
curl -fsSLO https://github.com/fursyt12/vMixRTC/releases/download/repo-x86_64/vmixrtc-repo.sh
less vmixrtc-repo.sh      # optional
sudo bash vmixrtc-repo.sh # add the repository, then: sudo pacman -S vmixrtc-bin
sudo bash vmixrtc-repo.sh --remove   # undo everything
```

The script writes `/etc/pacman.d/vmixrtc.conf`, adds a single
`Include = /etc/pacman.d/vmixrtc.conf` line to `/etc/pacman.conf` (after backing it up) and runs
`pacman -Sy`. Doing it by hand is just as fine — append to `/etc/pacman.conf`:

```ini
[vmixrtc]
SigLevel = Optional TrustAll
Server = https://github.com/fursyt12/vMixRTC/releases/download/repo-x86_64
```

| platform | how |
|---|---|
| **Arch Linux** | the command above |
| **Arch Linux, from source** | `cd packaging/arch/vmixrtc && makepkg -si` |
| **macOS** | `.dmg` from [Releases](https://github.com/fursyt12/vMixRTC/releases): `arm64` for Apple Silicon, `x86_64` for Intel — see [`INSTALL-macOS.md`](INSTALL-macOS.md) |
| **Windows** | `.msi`/`.exe` from Releases (WebView2 is preinstalled on Windows 10/11) |
| **Linux (other)** | `.deb` or `.AppImage` from Releases |

AUR packages are prepared in [`packaging/arch/`](packaging/arch) too (`vmixrtc`, `vmixrtc-bin`), but
AUR account registration is temporarily closed, so the pacman repository above is the primary way to
install on Arch.

## Opening controllers

**Open** in the toolbar asks the system file dialog for a `.vmc` (extension filter included),
so there is no need to type a path; the path field stays available for pasting one directly.
**Save** writes back to the opened file, and for a new document it asks where to save.

## What it can do

| area | state |
|---|---|
| `.vmc` controllers | read/write without losses (verified: read → write → read on all example files) |
| widgets | region, button, new button, text, score, timer, list, playlist, external data, clock, volume, T-Bar, variable viewer, container, MIDI, Stream Deck |
| scripts | expressions (`_('path')`, variables, `getvalue`/`split`/`len`), conditions, `Else`/`EndIf`, `GoTo`, `Timer`/`Delay`, `ExecLink`, `SetVariable`/`SetGlobalVariable`, `ValueChanged`, `IsPressed`, `HasVariable`, `API`/`APIPOST`, page commands, import/export, GUI editor |
| state | 1 Hz polling of the vMix API, per-widget active-state highlighting (`ActiveStateXPath`) |
| data providers | XML, JSON, Excel, Google Sheets, NDI sources, files, HTTP(S) with headers; rows → vMix titles — how-to: [`docs/DATA.md`](docs/DATA.md) |
| NDI | video receive through the NDI runtime (FFI), streamed to the UI as MJPEG |
| MIDI | `midir` input, learn mode, mappings to widget links |
| Stream Deck | direct HID access (no Elgato plugin needed), models, key events, brightness, learn |
| links | `Hotkey.Link` dispatch: MIDI, Stream Deck, schedule and other widgets all trigger the same actions |
| scheduler | clock events: time + weekdays + link, once per day |
| localization | Russian and English (`ui/locales/*.json`), switchable at runtime |

## Platforms

* **Linux** (x86_64, ARM64), **Windows** (x86_64), **macOS** (separate native builds: arm64 for
  Apple Silicon and x86_64 for Intel) — desktop.
* **Android** and **iOS** — Tauri v2 mobile targets (MIDI/Stream Deck are desktop-only and report
  that honestly on mobile).

Prebuilt binaries are produced by the CI workflow
[`.github/workflows/rust.yml`](.github/workflows/rust.yml): Windows `.msi`/`.exe`, macOS `.dmg` (arm64 and Intel), Linux `.deb`/`.AppImage`, plus the `vmixrtc-cli` command-line tool for every platform.
Push a tag `rust-v0.1.8` (or run the workflow manually) to build and publish a release.

**macOS users:** see [`INSTALL-macOS.md`](INSTALL-macOS.md) — pick the build for your chip (arm64 or
x86_64); it is ad-hoc signed by default, so the first launch needs the usual Gatekeeper confirmation (the Homebrew cask
handles it automatically). Developer ID signing and notarization switch on by themselves once the
`APPLE_*` repository secrets are set.

## Build

```bash
cargo build --release -p vmixrtc        # GUI
cargo build --release -p vmixrtc-cli    # CLI
cargo test --workspace                  # 121 tests
```

Everything lives in the repository root: `crates/` (the workspace), `examples/` (real `.vmc`
controllers) and `data/` (the vMix function catalogues). Prerequisites and per-platform
instructions (including cross-compilation and mobile) are in [`BUILD.md`](BUILD.md). Run the GUI
with a controller file:

```bash
./target/release/vmixrtc path/to/controller.vmc [widget-index] [script|rows|midi|deck|schedule]
```

## Command line

```bash
vmixrtc-cli status                                  # vMix version and state summary
vmixrtc-cli call Cut --param Input=1                # call a vMix function
vmixrtc-cli vmc examples/Scoreboard.vmc             # inspect a controller file
vmixrtc-cli press examples/Scoreboard.vmc 12        # run a widget script without the UI
vmixrtc-cli demo my.vmc --kind clock                # generate a demo controller
vmixrtc-cli verify examples --verbose               # verify the port against real files
```

## Documentation

| file | contents |
|---|---|
| [`PORTING.md`](PORTING.md) | how the original C# application was ported, module by module |
| [`PORTING-GAPS.md`](PORTING-GAPS.md) | what is not ported yet, and what to replace it with |
| [`PORTING-VERIFY.md`](PORTING-VERIFY.md) | how the port is verified (round-trip, catalogue checks, tests) |
| [`BUILD.md`](BUILD.md) | build and cross-compilation instructions |

## Documentation

* [`docs/DATA.md`](docs/DATA.md) — data sources: Google Sheets, Excel, HTTP API, XML, JSON, NDI
  and hand-written list items.
* [`BUILD.md`](BUILD.md) — building on Linux/macOS/Windows/mobile, icons, packaging.
* [`INSTALL-macOS.md`](INSTALL-macOS.md) — macOS install and Gatekeeper.
* [`packaging/arch/README.md`](packaging/arch/README.md) — Arch packages and the pacman repository.

## Verification

```bash
./target/release/vmixrtc-cli verify examples --verbose
```

Reports, for every example controller, whether the file survives a read/write round trip and whether
every widget type, command and provider is recognized: currently **6 files, 57 widgets, 45 commands,
0 problems**.

## License and origins

vMixRTC is released under the [MIT License](LICENSE): © 2026 fursyt12.

This is a fork of [vMixUTC](https://github.com/elgarf/vMixUTC) by elgarf; the original C#/WPF code
was removed from this repository and the Rust rewrite is the project. The upstream repository
carries no license file, so if you intend to redistribute the upstream work, check its terms with
the upstream author.
