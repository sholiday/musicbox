# Musicbox User Guide

Musicbox is an NFC-triggered audio player designed for Raspberry Pi–class devices. Each NFC card maps to a track (or playlist) inside a configurable music library. When someone taps a tagged card on the connected reader, Musicbox looks up the card's UID and plays the assigned audio file through the selected backend.

The application is written in Rust and ships with optional features that let the same binary serve development laptops, automated test rigs, and fully equipped Raspberry Pi deployments. This guide walks through installation, configuration, and day-to-day operation.

## Core Concepts

- **Library:** A TOML file enumerating card-to-track assignments backed by a `music_dir`. The controller keeps this mapping in memory while the app runs.
- **Cards:** NFC tags or cards identified by a hex UID. When a card is presented, Musicbox resolves the UID to a track and requests playback.
- **Readers:** Implementations of the `NfcReader` trait. The default build ships with a noop reader for laptops; enabling the `nfc-pcsc` feature activates the ACR122U-compatible PC/SC backend.
- **Audio players:** Implementations of the `AudioPlayer` trait. The `audio-rodio` feature enables the Rodio/CPAL player; otherwise the app runs in silent stub mode.
- **Telemetry:** Metrics and status are exposed via logs, and optionally through the `debug-http` web surface when that feature is enabled.

## Requirements

- Rust toolchain (stable channel) on the development machine.
- Raspberry Pi 2 or 3 running the 32-bit Raspberry Pi OS for deployment.
- ACR122U (or compatible) NFC USB reader when using the `nfc-pcsc` feature.
- ALSA-compatible audio output on the Pi when using the `audio-rodio` feature.
- Optional: Waveshare e-ink display HAT when using the `waveshare-display` feature.

## Quick Start on a Development Machine

1. Install Rust if needed:
   ```bash
   curl https://sh.rustup.rs -sSf | sh
   ```
2. Clone the repository and enter the workspace.
3. Run the binary with the noop reader and silent audio backend (no hardware required):
   ```bash
   cargo run -- config/demo.toml
   ```
   Use `--silent` to suppress playback, or omit it to exercise Rodio when the `audio-rodio` feature is enabled.

Running `cargo test` executes the unit tests with the default (noop) reader and silent audio backend. Add feature flags to exercise hardware integrations:

```bash
cargo test --features "audio-rodio nfc-pcsc"
```

## Installing on Raspberry Pi

### 1. Prepare the Development Host

Add the cross-compilation target once:

```bash
rustup target add armv7-unknown-linux-gnueabihf
```

Ensure the required native dependencies for ALSA and PC/SC development headers are installed on the host toolchain if you plan to enable those features. Refer to your distribution's packages (`libasound2-dev`, `libpcsclite-dev`, etc.).

### 2. Build Raspberry Pi Artifacts

Use the provided helper script to cross-compile. The script wires up the correct `pkg-config` paths for Rodio and PC/SC:

```bash
CARGO_FEATURES="audio-rodio nfc-pcsc debug-http" scripts/build-armv7.sh --release
```

Artifacts land under `target/armv7-unknown-linux-gnueabihf/release/`. Adjust the `CARGO_FEATURES` list to include only the integrations you need:

- `audio-rodio` for ALSA audio playback.
- `nfc-pcsc` for the USB NFC reader.
- `debug-http` for the Axum-based status server.
- `waveshare-display` to drive the Waveshare e-ink display.

### 3. Provision the Raspberry Pi

1. Create a directory layout for binaries, configuration, and media:
   ```bash
   mkdir -p ~/musicbox/bin ~/musicbox/config ~/musicbox/music
   ```
2. Copy the compiled binary, configuration, and media files to the Pi:
   ```bash
   scp target/armv7-unknown-linux-gnueabihf/release/musicbox pi@HOST:~/musicbox/bin/
   scp examples/config.example.toml pi@HOST:~/musicbox/config/musicbox.toml
   rsync -av songs/ pi@HOST:~/musicbox/music/
   ```
3. Confirm the NFC reader and audio hardware are connected. Install system packages if they are missing:
   ```bash
   sudo apt update
   sudo apt install -y libpcsclite1 pcscd alsa-utils
   sudo systemctl enable --now pcscd
   ```
4. Start the application manually:
   ```bash
   cd ~/musicbox
   ./bin/musicbox --reader pcsc --silent ./config/musicbox.toml
   ```
   Remove `--silent` to enable playback once audio hardware is in place.

To keep Musicbox running across reboots, create a `systemd` service that executes the same command and watches the card library directory.

## Configuring the Library

Consolidate card mappings in a TOML file. The repository includes `examples/config.example.toml` as a template:

```toml
music_dir = "/home/pi/music"

[cards]
"04a0b1c2d3" = "song1.mp3"
"abcd1234" = "album/track02.ogg"
```

- `music_dir` is the root directory for all audio files. Tracks are resolved relative to this path.
- Each entry under `[cards]` maps a hex-encoded card UID (no spaces, lowercase or uppercase) to a relative path in the music directory. Entries can point to subdirectories to group albums.
- Keep the file readable by Musicbox; the config loader validates syntax on startup. When the app starts, it aborts if the config file is missing or the referenced track is not accessible.

Store the configuration on the Raspberry Pi (e.g., `~/musicbox/config/musicbox.toml`) and update it whenever you add new tracks or cards. After editing, restart the Musicbox process or trigger a config reload if the running build exposes that capability.

## Managing NFC Cards

Musicbox offers CLI helpers to tag cards and update the config safely:

### Adding a New Card with a Reader

1. Connect the NFC reader to either your development machine (with `nfc-pcsc` enabled) or the Raspberry Pi.
2. Run the `add` subcommand with the target track:
   ```bash
   ./bin/musicbox add \
     --config ./config/musicbox.toml \
     --track tracks/lullaby.mp3
   ```
3. Present the blank card to the reader when prompted. Musicbox will:
   - Read the card UID.
   - Append or update the entry in the TOML config.
   - Optionally write metadata back to the NFC tag (skipable via `--skip-tag-write`).

Use `--reader` to override the backend (`pcsc`, `noop`, or `auto`) and `--poll-interval-ms` to customize how frequently the reader checks for new cards while waiting.

### Adding a Card Without a Reader

Pass the UID explicitly when you already know the card's value:

```bash
./bin/musicbox add \
  --config ./config/musicbox.toml \
  --track tracks/lullaby.mp3 \
  --card deadbeef
```

To run this command on hardware without a reader, pass `--reader noop`. Musicbox logs a reminder that the noop reader cannot verify the tag, but it still writes the mapping to the config.

### Manual Playback for Testing

Use the `manual trigger` subcommand to simulate a scan:

```bash
./bin/musicbox manual trigger \
  --config ./config/musicbox.toml \
  deadbeef
```

This is useful on the Pi before wiring up the reader or when you want to confirm the audio pipeline with a known UID.

## Running the Player

Launch the main loop by providing a config path:

```bash
./bin/musicbox \
  --poll-interval-ms 200 \
  --reader auto \
  --debug-http 0.0.0.0:3000 \
  ./config/musicbox.toml
```

- `--poll-interval-ms` adjusts how frequently the NFC reader checks for new cards. Higher values reduce CPU usage at the expense of responsiveness.
- `--reader` selects the backend (`auto`, `pcsc`, or `noop`). The default `auto` tries PC/SC first.
- `--silent` keeps the controller active without emitting audio; helpful for test rigs.
- `--debug-http` (feature-gated) exposes an Axum server for status dashboards and JSON diagnostics.
- Waveshare display options (`--waveshare-display`, `--waveshare-spi`, etc.) are available when the `waveshare-display` feature is compiled in.

Keep the binary and config together; the process logs to stdout/stderr. Use `journalctl` or your process supervisor to inspect logs on the Pi.

## Updating and Maintenance

- Rebuild and redeploy the binary whenever you update features or dependencies.
- Back up the TOML config regularly—the file is the source of truth for card assignments.
- Monitor `pcscd` and ALSA services on the Raspberry Pi if reader or audio failures occur.
- Use `cargo test --all-features` on the development machine before shipping changes to ensure integrations compile.

## Troubleshooting

- **Music does not play:** Verify that `audio-rodio` is enabled, ALSA libraries are installed, and the audio device is accessible. Run the binary without `--silent`.
- **No cards detected:** Confirm the `nfc-pcsc` feature is compiled in, `pcscd` is running, and the reader is visible under `lsusb`. Use `--reader noop` temporarily to verify the rest of the pipeline.
- **Config errors on startup:** Run `tomlq` or another linter, or simply inspect the reported line number in the error message. Ensure the track paths exist under `music_dir`.
- **Debug HTTP surface unavailable:** Double-check that the binary was compiled with `debug-http` and that the selected address is reachable from your network.

For deeper diagnostics, rebuild with `RUST_LOG=musicbox=debug` and inspect the structured logs.

