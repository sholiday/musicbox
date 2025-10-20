#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/.. && pwd)"

TARGET="armv7-unknown-linux-gnueabihf"
CARGO_FEATURES=${CARGO_FEATURES:-}

export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_PATH=/usr/lib/arm-linux-gnueabihf/pkgconfig
export PKG_CONFIG_LIBDIR=/usr/lib/arm-linux-gnueabihf/pkgconfig
export PKG_CONFIG_SYSROOT_DIR=/usr/arm-linux-gnueabihf

cd "$REPO_ROOT"
FEATURE_ARGS=()
if [[ -n "$CARGO_FEATURES" ]]; then
  FEATURE_ARGS=(--features "$CARGO_FEATURES")
else
  FEATURE_ARGS=(--all-features)
fi

cargo build --target "$TARGET" "${FEATURE_ARGS[@]}" "$@"
