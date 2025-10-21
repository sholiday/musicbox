# Troubleshooting

- **Music does not play:** Ensure the binary was built with `audio-rodio`, ALSA libraries are installed, and the audio device is accessible. Remove `--silent` while testing.
- **No cards detected:** Verify that the `nfc-pcsc` feature is compiled in, `pcscd` is running, and the reader appears in `lsusb`. Temporarily switch to `--reader noop` to confirm the rest of the pipeline.
- **Config errors on startup:** Inspect the reported line number, validate the TOML syntax, and confirm track paths exist under `music_dir`.
- **Debug HTTP surface unavailable:** Rebuild with the `debug-http` feature and ensure the chosen bind address is reachable from your network.

For deeper diagnostics, rebuild with `RUST_LOG=musicbox=debug` and inspect the structured logs.
