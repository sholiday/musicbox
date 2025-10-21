# Setup

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
   Pass `--silent` to suppress playback, or omit it to exercise Rodio when the `audio-rodio` feature is enabled.

Running `cargo test` executes the unit tests with the default noop reader and silent audio backend. Add feature flags to exercise hardware integrations:

```bash
cargo test --features "audio-rodio nfc-pcsc"
```
