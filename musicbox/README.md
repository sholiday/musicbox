# Musicbox Development Notes

This project is an NFC‑triggered music player aimed at running on a Raspberry Pi (32‑bit OS on Pi 2/3). It is being built with TDD and commits should remain small and frequent.

## Current Architecture

- `controller`: Pure domain logic mapping card UIDs to tracks. Works with any `AudioPlayer` implementation.
- `config`: Loads card→track mappings from a TOML file and produces a `Library`.
- `audio`: Optional backends implementing `AudioPlayer`. `RodioPlayer` is enabled via the `audio-rodio` Cargo feature; otherwise a silent stub is available.
- `reader`: Defines the `NfcReader` trait. A PC/SC implementation behind the `nfc-pcsc` feature polls an attached ACR122U reader; a noop reader is used otherwise.
- `app`: Glue code that loads config, wires the controller to a reader, and runs the event loop with callback hooks.
- `main`: CLI entry point built on clap. Allows selecting reader backend, poll interval, config path, and silent mode.

## Key Commands

```bash
# Run tests (default features only; no audio/NFC backends required)
cargo test

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

## Raspberry Pi Targets

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
