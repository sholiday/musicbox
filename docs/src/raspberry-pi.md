# Raspberry Pi Deployment

Follow these steps to build Musicbox on your development host and deploy to a Raspberry Pi.

## Prepare the Development Host

Add the cross-compilation target once:

```bash
rustup target add armv7-unknown-linux-gnueabihf
# For newer Raspberry Pi systems running a 64-bit OS:
rustup target add aarch64-unknown-linux-gnu
```

Install native dependencies required by optional features (ALSA and PC/SC headers) if you plan to enable them. Check your platform’s package manager for packages such as `libasound2-dev` and `libpcsclite-dev`.

On Debian/Ubuntu hosts, the 64-bit Raspberry Pi target commonly needs the cross linker and arm64 development packages via multiarch:

```bash
sudo dpkg --add-architecture arm64
sudo apt update
sudo apt install gcc-aarch64-linux-gnu libasound2-dev:arm64 libpcsclite-dev:arm64
```

The build scripts expect pkg-config files under `/usr/lib/<arch>/pkgconfig` and sysroot prefixes under `/usr/<arch>`. A dedicated sysroot works too, but keep `.cargo/config.toml` and the matching build script in sync if those paths differ on your host.

## Build Raspberry Pi Artifacts

Use the helper script to compile the project for `armv7-unknown-linux-gnueabihf`. The script wires up `pkg-config` so Rodio and PC/SC link correctly:

```bash
CARGO_FEATURES="audio-rodio nfc-pcsc debug-http" scripts/build-armv7.sh --release
```

Artifacts land under `target/armv7-unknown-linux-gnueabihf/release/`. Adjust `CARGO_FEATURES` to match your deployment:

For newer Raspberry Pi deployments running a 64-bit OS, build for `aarch64-unknown-linux-gnu` instead:

```bash
CARGO_FEATURES="audio-rodio nfc-pcsc debug-http waveshare-display" scripts/build-aarch64.sh --release
```

Artifacts land under `target/aarch64-unknown-linux-gnu/release/`.

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

## Run Musicbox with systemd

An example unit file lives at `examples/musicbox.service`. It assumes that the binary, configs, and media live under `/home/musicbox/musicbox/` and that a dedicated `musicbox` user belongs to the `audio` group.

1. Copy and edit the unit as needed:
   ```bash
   sudo cp examples/musicbox.service /etc/systemd/system/musicbox.service
   sudo cp examples/musicbox.logrotate /etc/logrotate.d/musicbox
   sudoedit /etc/systemd/system/musicbox.service
   ```
2. Optionally add overrides in `/etc/default/musicbox` (one `KEY=value` per line). Typical entries:
   ```bash
   MUSICBOX_CONFIG=/home/musicbox/musicbox/config/musicbox.toml
   MUSICBOX_READER=pcsc
   MUSICBOX_EXTRA_ARGS=--poll-interval-ms 200 --silent
   ```
3. Reload systemd, enable the service, and start it:
   ```bash
   sudo systemctl daemon-reload
   sudo systemctl enable --now musicbox.service
   ```

The unit restarts automatically on failure and streams stdout/stderr both to journald (`journalctl -u musicbox`) and to `/var/log/musicbox/musicbox.log`, giving you a persistent log even after reboots. The sample logrotate rule caps that file by rotating it daily or when it reaches 10 MiB.

### Authorize PC/SC reader access

When `musicbox` runs as a headless service, `pcscd` may reject it with `SecurityViolation` errors unless polkit explicitly grants access. The example unit therefore joins the service account to the `musicboxd` group; authorize that group once per Pi:

```bash
sudo groupadd --system musicboxd  # no-op if it already exists
sudo usermod -a -G musicboxd musicbox  # replace with the user defined in the unit
sudo tee /etc/polkit-1/rules.d/49-musicbox.rules >/dev/null <<'EOF'
polkit.addRule(function(action, subject) {
  if ((action.id == "org.debian.pcsc-lite.access_pcsc" ||
       action.id == "org.debian.pcsc-lite.access_card") &&
      subject.isInGroup("musicboxd")) {
    return polkit.Result.YES;
  }
});
EOF
sudo systemctl restart pcscd.service musicbox.service
```

After the restart, `journalctl -u musicbox` should show the hardware reader being detected instead of falling back to the noop backend.
