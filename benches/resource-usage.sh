#!/usr/bin/env bash
# Peak RSS / timing for warm-keep (or a no-op install) via /usr/bin/time.
#
# Usage: ./benches/resource-usage.sh [fixture-name] [with-dev]
# Default fixture: laravel-skeleton (typically --no-dev).
#
# Dev packages: pass `with-dev` or set PUCK_BENCH_DEV=1 to install require-dev
# (required for fair laravel-app RSS vs skeleton --no-dev).
#
# Prints wall/user/sys and peak RSS when the host time(1) supports it.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FIXTURE_NAME="${1:-laravel-skeleton}"
DEV_MODE=0
if [[ "${2:-}" == "with-dev" || "${PUCK_BENCH_DEV:-}" == "1" ]]; then
  DEV_MODE=1
fi
FIXTURE="$ROOT/fixtures/$FIXTURE_NAME"
BIN="$ROOT/target/release/puck"

if [[ ! -f "$FIXTURE/composer.lock" ]]; then
  echo "resource-usage: missing $FIXTURE/composer.lock" >&2
  exit 2
fi

PUCK_DEV_ARGS=()
DEV_LABEL="--no-dev"
if [[ "$DEV_MODE" -eq 1 ]]; then
  DEV_LABEL="with require-dev (--dev)"
else
  PUCK_DEV_ARGS+=(--no-dev)
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
"$BIN" install --working-dir "$WORKDIR" "${PUCK_DEV_ARGS[@]}" --no-scripts >/dev/null

TIME_BIN="/usr/bin/time"
if [[ ! -x "$TIME_BIN" ]]; then
  TIME_BIN="$(command -v time || true)"
fi
if [[ -z "${TIME_BIN}" ]]; then
  echo "resource-usage: no time(1) found; running without RSS" >&2
  "$BIN" install --working-dir "$WORKDIR" "${PUCK_DEV_ARGS[@]}" --no-scripts
  exit 0
fi

# macOS: -l ; GNU: -v
OUT="$(mktemp)"
set +e
if "$TIME_BIN" -l true >/dev/null 2>&1; then
  "$TIME_BIN" -l "$BIN" install --working-dir "$WORKDIR" "${PUCK_DEV_ARGS[@]}" --no-scripts >"$OUT" 2>&1
  STATUS=$?
  MODE=macos
elif "$TIME_BIN" -v true >/dev/null 2>&1; then
  "$TIME_BIN" -v "$BIN" install --working-dir "$WORKDIR" "${PUCK_DEV_ARGS[@]}" --no-scripts >"$OUT" 2>&1
  STATUS=$?
  MODE=gnu
else
  "$BIN" install --working-dir "$WORKDIR" "${PUCK_DEV_ARGS[@]}" --no-scripts >"$OUT" 2>&1
  STATUS=$?
  MODE=plain
fi
set -e

echo "resource-usage: fixture=$FIXTURE_NAME mode=$MODE exit=$STATUS ($DEV_LABEL)"
if [[ "$MODE" == "macos" ]]; then
  # Sample lines from time -l
  grep -E 'real|user|sys|maximum resident set size' "$OUT" || cat "$OUT"
elif [[ "$MODE" == "gnu" ]]; then
  grep -E 'Elapsed \(wall clock\)|User time|System time|Maximum resident set size' "$OUT" || cat "$OUT"
else
  cat "$OUT"
fi
