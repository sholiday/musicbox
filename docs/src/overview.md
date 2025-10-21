# Overview

Musicbox is an NFC-triggered audio player built for Raspberry Pi–class devices. Each NFC card maps to a track (or playlist) inside a configurable music library. When someone taps a tagged card on the connected reader, Musicbox resolves the card UID, selects the matching track, and requests playback through the active audio backend.

The project is written in Rust and leans on optional features so the same binary can support development laptops, automated test rigs, and production Raspberry Pi deployments. This book provides a guided tour covering installation, configuration, and day-to-day usage.

## Core Concepts

- **Library:** A TOML file enumerating card-to-track assignments backed by a `music_dir`. The controller keeps this mapping in memory while the app runs.
- **Cards:** NFC tags identified by a hex UID. When a card is presented, Musicbox looks up the UID and either plays the configured track or reports an error if the card is unknown.
- **Readers:** Implementations of the `NfcReader` trait. The default build ships with a noop reader for laptops; enabling the `nfc-pcsc` feature activates the ACR122U-compatible PC/SC backend.
- **Audio players:** Implementations of the `AudioPlayer` trait. The `audio-rodio` feature enables the Rodio/CPAL player; otherwise the app runs in silent stub mode.
- **Telemetry:** Structured logs and optional HTTP diagnostics (via the `debug-http` feature) provide insight into system health without attaching a debugger to the Raspberry Pi.
