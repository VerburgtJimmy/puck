#!/usr/bin/env bash
# Peak RSS / timing for warm-keep (or a no-op install) via /usr/bin/time.
#
# Usage: ./benches/resource-usage.sh [fixture-name]
# Default fixture: laravel-skeleton
#
# Prints wall/user/sys and peak RSS when the host time(1) supports it.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FIXTURE_NAME="${1:-laravel-skeleton}"
FIXTURE="$ROOT/fixtures/$FIXTURE_NAME"
BIN="$ROOT/target/release/puck"

if [[ ! -f "$FIXTURE/composer.lock" ]]; then
  echo "resource-usage: missing $FIXTURE/composer.lock" >&2
  exit 2
fi

# Prefer plain cargo (rust-toolchain.toml / CI default); rustup run as fallback.
run_cargo() {
  if command -v cargo >/dev/null 2>&1; then
    cargo "$@"
  elif command -v rustup >/dev/null 2>&1; then
    rustup run 1.96.0 cargo "$@"
  else
    echo "resource-usage: cargo not found" >&2
    exit 2
  fi
}

cd "$ROOT"
if [[ ! -x "$BIN" ]]; then
  echo "resource-usage: building release puck..." >&2
  run_cargo build --release -p puck_cli
fi

WORKDIR="$(mktemp -d "$ROOT/.tmp-rss.XXXXXX")"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

cp "$FIXTURE/composer.json" "$FIXTURE/composer.lock" "$WORKDIR/"
# Warm vendor once so the measured run is warm-keep / near no-op.
"$BIN" install --working-dir "$WORKDIR" --no-dev --no-scripts >/dev/null

TIME_BIN="/usr/bin/time"
if [[ ! -x "$TIME_BIN" ]]; then
  TIME_BIN="$(command -v time || true)"
fi
if [[ -z "${TIME_BIN}" ]]; then
  echo "resource-usage: no time(1) found; running without RSS" >&2
  "$BIN" install --working-dir "$WORKDIR" --no-dev --no-scripts
  exit 0
fi

# macOS: -l ; GNU: -v
OUT="$(mktemp)"
set +e
if "$TIME_BIN" -l true >/dev/null 2>&1; then
  "$TIME_BIN" -l "$BIN" install --working-dir "$WORKDIR" --no-dev --no-scripts >"$OUT" 2>&1
  STATUS=$?
  MODE=macos
elif "$TIME_BIN" -v true >/dev/null 2>&1; then
  "$TIME_BIN" -v "$BIN" install --working-dir "$WORKDIR" --no-dev --no-scripts >"$OUT" 2>&1
  STATUS=$?
  MODE=gnu
else
  "$BIN" install --working-dir "$WORKDIR" --no-dev --no-scripts >"$OUT" 2>&1
  STATUS=$?
  MODE=plain
fi
set -e

echo "resource-usage: fixture=$FIXTURE_NAME mode=$MODE exit=$STATUS"
if [[ "$MODE" == "macos" ]]; then
  # Sample lines from time -l
  grep -E 'real|user|sys|maximum resident set size' "$OUT" || cat "$OUT"
elif [[ "$MODE" == "gnu" ]]; then
  grep -E 'Elapsed \(wall clock\)|User time|System time|Maximum resident set size' "$OUT" || cat "$OUT"
else
  cat "$OUT"
fi
