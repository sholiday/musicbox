# Raspberry Pi Deployment

Follow these steps to build Musicbox on your development host and deploy to a Raspberry Pi.

## Prepare the Development Host

Add the cross-compilation target once:

```bash
rustup target add armv7-unknown-linux-gnueabihf
```

Install native dependencies required by optional features (ALSA and PC/SC headers) if you plan to enable them. Check your platform’s package manager for packages such as `libasound2-dev` and `libpcsclite-dev`.

## Build Raspberry Pi Artifacts

Use the helper script to compile the project for `armv7-unknown-linux-gnueabihf`. The script wires up `pkg-config` so Rodio and PC/SC link correctly:

```bash
CARGO_FEATURES="audio-rodio nfc-pcsc debug-http" scripts/build-armv7.sh --release
```

Artifacts land under `target/armv7-unknown-linux-gnueabihf/release/`. Adjust `CARGO_FEATURES` to match your deployment:

- `audio-rodio` for ALSA playback.
- `nfc-pcsc` for the USB NFC reader.
- `debug-http` for the Axum-based status server.
- `waveshare-display` for the Waveshare e-ink display HAT.

## Provision the Raspberry Pi

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
3. Confirm the NFC reader and audio hardware are connected. Install supporting packages if they are missing:
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

To keep Musicbox running across reboots, convert the launch command into a `systemd` service or integrate it with your chosen process supervisor.
