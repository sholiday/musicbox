#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/.. && pwd)"

CARGO_FEATURES=${CARGO_FEATURES:-}
CONFIG_PATH=${1:-"$REPO_ROOT/examples/config.example.toml"}

echo "Running musicbox with config: $CONFIG_PATH"
echo "Cargo features: ${CARGO_FEATURES:-<none>}"

cd "$REPO_ROOT"
cargo run ${CARGO_FEATURES:+--features "$CARGO_FEATURES"} -- \
  --reader auto \
  --poll-interval-ms 500 \
  --silent \
  "$CONFIG_PATH"
