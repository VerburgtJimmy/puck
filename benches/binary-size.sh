#!/usr/bin/env bash
# Print release puck binary size (builds if missing/stale sources).
#
# Usage: ./benches/binary-size.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/release/puck"

cd "$ROOT"
if [[ ! -x "$BIN" ]] || [[ crates/puck_cli/src/main.rs -nt "$BIN" ]]; then
  echo "binary-size: building release puck..." >&2
  rustup run 1.96.0 cargo build --release -p puck_cli
fi

BYTES="$(wc -c < "$BIN" | tr -d ' ')"
HUMAN="$(ls -lh "$BIN" | awk '{print $5}')"
echo "puck release binary: $HUMAN ($BYTES bytes)"
echo "path: $BIN"
