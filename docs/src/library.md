# Library Configuration

Musicbox loads its card-to-track assignments from a TOML configuration file. The repository includes `examples/config.example.toml` as a template:

```toml
music_dir = "/home/pi/music"

[cards]
"04a0b1c2d3" = "song1.mp3"
"abcd1234" = "album/track02.ogg"
```

- `music_dir` points at the root directory containing your audio files. Track paths resolve relative to this directory.
- Each key under `[cards]` is a hex-encoded card UID (no spaces, either case). Values are paths to playable audio files under `music_dir`.
- Paths can reference subdirectories. Keep directory names descriptive if you plan to group albums or playlists.

Store the configuration on the Raspberry Pi (for example, `~/musicbox/config/musicbox.toml`). Update the file whenever you add new tracks or cards, then restart the Musicbox service or trigger a config reload if available. The loader validates syntax and track paths on startup; the process exits with a descriptive error if validation fails.
