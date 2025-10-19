# Musicbox Development Notes

This project is an NFC‑triggered music player aimed at running on a Raspberry Pi (32‑bit OS on Pi 2/3). It is being built with TDD and commits should remain small and frequent.

## Current Architecture

- `controller`: Pure domain logic mapping card UIDs to tracks. Works with any `AudioPlayer` implementation so we can exercise it thoroughly in unit tests without bringing hardware along.
- `config`: Loads card→track mappings from a TOML file and produces a `Library`.
- `audio`: Optional backends implementing `AudioPlayer`. `RodioPlayer` is enabled via the `audio-rodio` Cargo feature; otherwise a silent stub is available, letting the app boot in CI or on dev laptops without ALSA.
- `reader`: Defines the `NfcReader` trait. A PC/SC implementation behind the `nfc-pcsc` feature polls an attached ACR122U reader; a noop reader is used otherwise so we can still run and observe telemetry on machines without the hardware.
- `app`: Glue code that loads config, wires the controller to a reader, and runs the event loop with callback hooks.
- `main`: CLI entry point built on clap. Allows selecting reader backend, poll interval, config path, and silent mode so the same binary can serve development, test rigs, and the Pi image.

## Key Commands

```bash
# Run tests (default features only; no audio/NFC backends required)
cargo test

# Cross-compile for Raspberry Pi (armv7)
scripts/build-armv7.sh --release
# Include optional features when needed
CARGO_FEATURES="audio-rodio nfc-pcsc" scripts/build-armv7.sh --release

# Run with Rodio audio support (requires system audio libs)
cargo test --features audio-rodio

# Run with PC/SC reader support (requires libpcsclite headers)
cargo test --features nfc-pcsc

# Combine features as needed
cargo test --features "audio-rodio nfc-pcsc"

# Include the optional debug HTTP server
cargo test --features "audio-rodio nfc-pcsc debug-http"
```

### Binary Usage

```bash
# Example using clap CLI options
cargo run --release --features "audio-rodio nfc-pcsc" -- \
  --poll-interval-ms 200 \
  --reader auto \
  --silent \
  /path/to/config.toml
```

Options:

- `CONFIG` (positional): path to the TOML config mapping card UIDs to tracks.
- `--poll-interval-ms`: adjust NFC polling interval (default `200` ms).
- `--reader {auto|pcsc|noop}`: force reader choice; `auto` tries PC/SC then falls back to noop.
- `--silent`: skip audio playback regardless of backend availability.
- `--debug-http <addr>` *(requires `debug-http` feature)*: expose telemetry via Axum (e.g. `127.0.0.1:3000`).

A starter config can be found in `examples/config.example.toml`.

### Why the optional features?

The binary needs to run in a few different contexts:

- **CI / developer laptops** – usually missing ALSA and PC/SC headers. The default build therefore avoids those dependencies so `cargo test` stays fast and hermetic.
- **Raspberry Pi image** – supply `--features "audio-rodio nfc-pcsc"` so the concrete Rodio player and PC/SC reader are compiled in and the hardware works at runtime.
- **Debug rigs** – when you want the Axum status surface, also enable `debug-http` and pass `--debug-http <addr>` on the CLI. The server lives on a separate thread to avoid blocking the reader loop.

Keeping these concerns behind feature flags lets us ship one codebase while still producing lightweight binaries for automated pipelines.

## Raspberry Pi Targets

- Build everything from your dev machine; the Pi only needs the deployed binaries. Use
  `scripts/build-armv7.sh` to produce the `armv7-unknown-linux-gnueabihf` artifacts and copy the
  resulting files under `target/armv7-unknown-linux-gnueabihf/{debug,release}` to the Pi (e.g.,
  via `rsync`). The script wires up the required `pkg-config` environment so ALSA and PC/SC
  libraries resolve correctly when optional features are enabled.
- For on-device testing without installing Rust, cross-compile the test harnesses with
  `scripts/build-armv7.sh --tests`, copy the executables from
  `target/armv7-unknown-linux-gnueabihf/debug/deps/` to the Pi, and run them there.
- Pi 2/3, standard 32‑bit Raspberry Pi OS.
- NFC reader: ACR122U (PC/SC).
- Audio: Raspberry Pi audio output via Rodio/CPAL (requires ALSA).

When cross-compiling, ensure the appropriate system libraries (`libpcsclite`, ALSA) are available in the target sysroot if the corresponding features are enabled.

## Development Practices

- Use tests to drive new behavior (`cargo test` runs quickly without hardware).
- Commit early and often with descriptive messages.
- Avoid mocks unless hardware interaction cannot be reasonably replicated.
- If external libraries are missing on the dev machine, prefer optional features so the default build stays portable.

## Next Steps (as of latest cadence)

1. Implement a real PC/SC reader loop on the Pi and validate with `--features nfc-pcsc`.
2. Integrate Rodio playback on the Pi (`--features audio-rodio`), handling error reporting for missing audio devices.
3. Extend configuration (TOML or CLI) to control logging/debug output and prep the Axum-based status UI.
