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
