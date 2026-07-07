#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/.. && pwd)"

TARGET="aarch64-unknown-linux-gnu"
CARGO_FEATURES=${CARGO_FEATURES:-}

export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_PATH=/usr/lib/aarch64-linux-gnu/pkgconfig
export PKG_CONFIG_LIBDIR=/usr/lib/aarch64-linux-gnu/pkgconfig
export PKG_CONFIG_SYSROOT_DIR=/usr/aarch64-linux-gnu

cd "$REPO_ROOT"
FEATURE_ARGS=()
if [[ -n "$CARGO_FEATURES" ]]; then
  FEATURE_ARGS=(--features "$CARGO_FEATURES")
else
  FEATURE_ARGS=(--all-features)
fi

cargo build --target "$TARGET" "${FEATURE_ARGS[@]}" "$@"
