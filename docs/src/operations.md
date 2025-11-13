# Running Musicbox

Start the main loop by providing a config path:

```bash
./bin/musicbox \
  --poll-interval-ms 200 \
  --reader auto \
  --debug-http 0.0.0.0:3000 \
  ./config/musicbox.toml
```

- `--poll-interval-ms` controls how frequently the NFC reader checks for new cards. Higher values reduce CPU load at the cost of responsiveness.
- `--reader` selects the backend (`auto`, `pcsc`, or `noop`). The default `auto` tries PC/SC first and falls back to noop.
- `--silent` keeps the controller active without emitting audio; helpful for test rigs or headless validation.
- `--debug-http` (feature-gated) exposes an Axum server for status dashboards and JSON diagnostics.
- Waveshare display options (`--waveshare-display`, `--waveshare-spi`, and related flags) become available when the binary is compiled with the `waveshare-display` feature.

The process logs to stdout/stderr. When running under `systemd`, use `journalctl -u musicbox` to review logs and confirm hardware interactions.

## Run Musicbox with systemd

Linux hosts can keep Musicbox running in the background by installing the sample unit defined in `examples/musicbox.service`.

1. Create a dedicated runtime user (for example `musicbox`) that belongs to the `audio` group, copy the unit, and adjust `User`, `Group`, `WorkingDirectory`, and the default environment variables so they match your filesystem layout:
   ```bash
   sudo useradd --system --create-home musicbox
   sudo usermod -a -G audio musicbox
   sudo cp examples/musicbox.service /etc/systemd/system/musicbox.service
   sudoedit /etc/systemd/system/musicbox.service
   ```
2. Optionally supply extra overrides in `/etc/default/musicbox` (one `KEY=value` per line). Common overrides:
   ```bash
   MUSICBOX_CONFIG=/home/musicbox/musicbox/config/musicbox.toml
   MUSICBOX_READER=pcsc
   MUSICBOX_EXTRA_ARGS=--poll-interval-ms 200 --silent
   ```
3. Reload `systemd`, enable the service, and start it:
   ```bash
   sudo systemctl daemon-reload
   sudo systemctl enable --now musicbox.service
   ```
4. Inspect status and logs to verify that the NFC reader and audio backend initialize correctly:
   ```bash
   systemctl status musicbox.service
   journalctl -u musicbox -f
   ```

The sample unit restarts automatically on failure, keeps recent logs in `journalctl`, and mirrors them to `/var/log/musicbox/musicbox.log` via `LogsDirectory` so you retain traces across reboots. Place configuration files and media under the `WorkingDirectory` to keep permissions straightforward, and ensure `pcscd` is enabled when using the USB reader (`sudo systemctl enable --now pcscd`).
