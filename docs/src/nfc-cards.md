# NFC Card Management

Musicbox provides CLI helpers for registering cards and updating the library safely.

## Adding a Card with a Reader

1. Connect the NFC reader to your development machine (with `nfc-pcsc` enabled) or to the Raspberry Pi.
2. Run the `add` subcommand with the target track:
   ```bash
   ./bin/musicbox add \
     --config ./config/musicbox.toml \
     --track tracks/lullaby.mp3
   ```
3. Present the blank card when prompted. Musicbox will read the card UID, append or update the entry in the TOML config, and optionally write metadata back to the tag. Pass `--skip-tag-write` to avoid programming the physical tag.

Override the reader backend with `--reader` (`pcsc`, `noop`, or `auto`) and adjust responsiveness with `--poll-interval-ms`.

## Adding a Card Without a Reader

Provide the UID explicitly when you already know the card value:

```bash
./bin/musicbox add \
  --config ./config/musicbox.toml \
  --track tracks/lullaby.mp3 \
  --card deadbeef \
  --reader noop
```

Musicbox logs a reminder that the noop reader cannot verify the tag, but it still writes the mapping to the config file.

## Manual Playback for Testing

Simulate a scan with the `manual trigger` subcommand:

```bash
./bin/musicbox manual trigger \
  --config ./config/musicbox.toml \
  deadbeef
```

This command confirms the controller can resolve a known UID and reach the audio backend before you connect real hardware.
