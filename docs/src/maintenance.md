# Maintenance

- Rebuild and redeploy the binary whenever dependencies or features change.
- Back up the TOML configuration regularly; it is the authoritative record of card assignments.
- Monitor `pcscd` and ALSA services on the Raspberry Pi if reader or audio failures occur.
- Run `cargo test --all-features` on the development machine before shipping changes to ensure optional integrations continue to compile.
