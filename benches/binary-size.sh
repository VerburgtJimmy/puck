#!/usr/bin/env bash
# Print release puck binary size (builds if missing/stale sources).
#
# Usage: ./benches/binary-size.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/release/puck"

# Prefer plain cargo (rust-toolchain.toml / CI default); rustup run as fallback.
run_cargo() {
  if command -v cargo >/dev/null 2>&1; then
    cargo "$@"
  elif command -v rustup >/dev/null 2>&1; then
    rustup run 1.96.0 cargo "$@"
  else
    echo "binary-size: cargo not found" >&2
    exit 2
  fi
}

cd "$ROOT"
if [[ ! -x "$BIN" ]] || [[ crates/puck_cli/src/main.rs -nt "$BIN" ]]; then
  echo "binary-size: building release puck..." >&2
  run_cargo build --release -p puck_cli
fi

BYTES="$(wc -c < "$BIN" | tr -d ' ')"
HUMAN="$(ls -lh "$BIN" | awk '{print $5}')"
echo "puck release binary: $HUMAN ($BYTES bytes)"
echo "path: $BIN"
