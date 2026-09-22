# vMixRTC — vMix Rust Title Controller

**vMixRTC** is a cross-platform controller for [vMix](https://vmix.com) written entirely in Rust:
title/scoreboard widgets, a script engine, data providers, MIDI, Stream Deck and an NDI monitor.

It is a full rewrite of [vMixUTC](https://github.com/elgarf/vMixUTC) (the original C#/WPF
application) — same `.vmc` controller files, same widget behaviour, but a single Rust codebase with
no .NET, no WPF and no Windows-only dependencies.

> The C#/WPF original has been removed from this repository. It stays available upstream at
> [elgarf/vMixUTC](https://github.com/elgarf/vMixUTC); this fork continues as the Rust project.

## What it can do

| area | state |
|---|---|
| `.vmc` controllers | read/write without losses (verified: read → write → read on all example files) |
| widgets | region, button, new button, text, score, timer, list, playlist, external data, clock, volume, T-Bar, variable viewer, container, MIDI, Stream Deck |
| scripts | expressions (`_('path')`, variables, `getvalue`/`split`/`len`), conditions, `Else`/`EndIf`, `GoTo`, `Timer`/`Delay`, `ExecLink`, `SetVariable`/`SetGlobalVariable`, `ValueChanged`, `IsPressed`, `HasVariable`, `API`/`APIPOST`, page commands, import/export, GUI editor |
| state | 1 Hz polling of the vMix API, per-widget active-state highlighting (`ActiveStateXPath`) |
| data providers | XML, JSON, Excel, Google Sheets, NDI sources, files, HTTP(S) with headers; rows → vMix titles |
| NDI | video receive through the NDI runtime (FFI), streamed to the UI as MJPEG |
| MIDI | `midir` input, learn mode, mappings to widget links |
| Stream Deck | direct HID access (no Elgato plugin needed), models, key events, brightness, learn |
| links | `Hotkey.Link` dispatch: MIDI, Stream Deck, schedule and other widgets all trigger the same actions |
| scheduler | clock events: time + weekdays + link, once per day |
| localization | Russian and English (`ui/locales/*.json`), switchable at runtime |

## Platforms

* **Linux** (x86_64, ARM64), **Windows** (x86_64), **macOS** (universal: Apple Silicon + Intel) —
  desktop.
* **Android** and **iOS** — Tauri v2 mobile targets (MIDI/Stream Deck are desktop-only and report
  that honestly on mobile).

Prebuilt binaries are produced by the CI workflow
[`.github/workflows/rust.yml`](.github/workflows/rust.yml): Windows `.msi`/`.exe`, macOS universal
`.dmg`, Linux `.deb`/`.AppImage`, plus the `vmixrtc-cli` command-line tool for every platform.
Push a tag `rust-v0.1.3` (or run the workflow manually) to build and publish a release.

**macOS users:** see [`INSTALL-macOS.md`](INSTALL-macOS.md) — the build is universal and ad-hoc
signed by default, so the first launch needs the usual Gatekeeper confirmation (the Homebrew cask
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

## Verification

```bash
./target/release/vmixrtc-cli verify examples --verbose
```

Reports, for every example controller, whether the file survives a read/write round trip and whether
every widget type, command and provider is recognized: currently **6 files, 57 widgets, 45 commands,
0 problems**.

## License and origins

This is a fork of [vMixUTC](https://github.com/elgarf/vMixUTC) by elgarf. The upstream repository
carries no license file; the Rust rewrite keeps the same relationship to it. If you redistribute
vMixRTC, check the upstream terms first.
