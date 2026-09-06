#!/usr/bin/env bash
# Cold/warm install timings: Composer vs puck on a fixture.
#
# Usage: ./benches/install-timing.sh [fixture-name]
# Default fixture: laravel-skeleton (always --no-dev).
#
# Prints a markdown table to stdout and appends/updates
# puck-notes benchmarks.md, or ./bench-out/benchmarks.md if unset/unwritable.
# Override with PUCK_NOTES_BENCHMARKS.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FIXTURE_NAME="${1:-laravel-skeleton}"
FIXTURE="$ROOT/fixtures/$FIXTURE_NAME"
NOTES_DOC="${PUCK_NOTES_BENCHMARKS:-$HOME/Developer/personal/puck-notes/docs/benchmarks.md}"

if [[ ! -f "$FIXTURE/composer.lock" ]]; then
  echo "timing: missing $FIXTURE/composer.lock" >&2
  exit 2
fi

resolve_composer() {
  if command -v composer >/dev/null 2>&1; then
    echo "composer"
    return
  fi
  local herd="/Users/jimmyverburgt/Library/Application Support/Herd/bin/composer"
  if [[ -x "$herd" ]]; then
    echo "$herd"
    return
  fi
  echo ""
}

COMPOSER_BIN="$(resolve_composer)"
if [[ -z "$COMPOSER_BIN" ]]; then
  echo "timing: host composer required (docker timings not implemented)" >&2
  exit 2
fi

WORKDIR="$(mktemp -d "$ROOT/.tmp-timing.XXXXXX")"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

prepare_tree() {
  local dest="$1"
  rm -rf "$dest"
  mkdir -p "$dest"
  cp "$FIXTURE/composer.json" "$FIXTURE/composer.lock" "$dest/"
  if [[ -d "$FIXTURE/bootstrap" ]]; then
    cp -R "$FIXTURE/bootstrap" "$dest/"
  fi
}

elapsed_ms() {
  # macOS / Linux portable-ish: python for wall clock
  local start="$1"
  local end
  end="$(python3 -c 'import time; print(time.time())')"
  python3 -c "print(int(($end - $start) * 1000))"
}

time_cmd_ms() {
  local start
  start="$(python3 -c 'import time; print(time.time())')"
  "$@" >/dev/null 2>&1
  elapsed_ms "$start"
}

run_puck() {
  local dir="$1"
  (
    cd "$ROOT"
    "$PUCK_BIN" install --working-dir "$dir" --no-dev --no-scripts
  )
}

run_composer() {
  local dir="$1"
  (
    cd "$dir"
    "$COMPOSER_BIN" install --no-dev --no-scripts --ignore-platform-reqs --no-interaction --no-ansi
  )
}

echo "timing: fixture=$FIXTURE_NAME (--no-dev --no-scripts)"
echo "timing: composer=$COMPOSER_BIN"

# Prefer plain cargo (rust-toolchain.toml / CI default); rustup run as fallback.
run_cargo() {
  if command -v cargo >/dev/null 2>&1; then
    cargo "$@"
  elif command -v rustup >/dev/null 2>&1; then
    rustup run 1.96.0 cargo "$@"
  else
    echo "timing: cargo not found" >&2
    exit 2
  fi
}

if [[ -n "${PUCK_BIN:-}" && -x "${PUCK_BIN}" ]]; then
  echo "timing: using PUCK_BIN=$PUCK_BIN"
else
  echo "timing: building release puck…"
  run_cargo build --release -q -p puck_cli
  TARGET_DIR="$(run_cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
  PUCK_BIN="$TARGET_DIR/release/puck"
  if [[ ! -x "$PUCK_BIN" ]]; then
    echo "timing: missing release binary at $PUCK_BIN" >&2
    exit 2
  fi
  echo "timing: puck=$PUCK_BIN"
fi

# --- Cold: empty vendor ---
COLD_C="$WORKDIR/cold-composer"
COLD_P="$WORKDIR/cold-puck"
prepare_tree "$COLD_C"
prepare_tree "$COLD_P"

echo "timing: cold composer…"
COLD_COMPOSER_MS="$(time_cmd_ms run_composer "$COLD_C")"
echo "timing: cold puck…"
COLD_PUCK_MS="$(time_cmd_ms run_puck "$COLD_P")"

# --- Warm: second install ---
# Fair comparisons:
# 1) vendor already present (typical up-to-date CI / local re-run)
# 2) wiped vendor with warm package cache / store
WARM_C="$WORKDIR/warm-composer"
WARM_P="$WORKDIR/warm-puck"
prepare_tree "$WARM_C"
prepare_tree "$WARM_P"

# Prime both
run_composer "$WARM_C" >/dev/null
run_puck "$WARM_P" >/dev/null

echo "timing: warm composer (vendor present)…"
WARM_COMPOSER_PRESENT_MS="$(time_cmd_ms run_composer "$WARM_C")"

echo "timing: warm puck (vendor present)…"
WARM_PUCK_PRESENT_MS="$(time_cmd_ms run_puck "$WARM_P")"

echo "timing: warm puck (store warm, wipe vendor)…"
rm -rf "$WARM_P/vendor"
WARM_PUCK_WIPE_MS="$(time_cmd_ms run_puck "$WARM_P")"

echo "timing: warm composer (wipe vendor, cache warm)…"
rm -rf "$WARM_C/vendor"
WARM_COMPOSER_CACHE_MS="$(time_cmd_ms run_composer "$WARM_C")"

DATE_UTC="$(date -u +"%Y-%m-%d %H:%M:%SZ")"
HOST="$(uname -s)/$(uname -m)"

TABLE=$(cat <<EOF
| scenario | composer (ms) | puck (ms) |
|---|---:|---:|
| cold (empty vendor) | ${COLD_COMPOSER_MS} | ${COLD_PUCK_MS} |
| warm (vendor present) | ${WARM_COMPOSER_PRESENT_MS} | ${WARM_PUCK_PRESENT_MS} |
| warm (wipe vendor, cache/store warm) | ${WARM_COMPOSER_CACHE_MS} | ${WARM_PUCK_WIPE_MS} |
EOF
)

echo
echo "## Install timing: \`${FIXTURE_NAME}\` --no-dev"
echo
echo "- when: ${DATE_UTC}"
echo "- host: ${HOST}"
echo "- composer: \`$( "$COMPOSER_BIN" --version 2>/dev/null | head -1)\`"
echo
echo "$TABLE"
echo

resolve_notes_doc() {
  local doc="$NOTES_DOC"
  local parent
  parent="$(dirname "$doc")"
  if [[ -z "${PUCK_NOTES_BENCHMARKS:-}" ]]; then
    if [[ ! -d "$parent" || ! -w "$parent" ]]; then
      doc="$ROOT/bench-out/benchmarks.md"
      parent="$(dirname "$doc")"
    fi
  fi
  if ! mkdir -p "$parent" 2>/dev/null || [[ ! -w "$parent" ]]; then
    echo ""
    return
  fi
  # Ensure the file itself is creatable / writable
  if [[ -e "$doc" && ! -w "$doc" ]]; then
    echo ""
    return
  fi
  echo "$doc"
}

NOTES_TARGET="$(resolve_notes_doc)"
if [[ -z "$NOTES_TARGET" ]]; then
  echo "timing: skipping notes append (path missing or not writable: $NOTES_DOC); stdout table above is authoritative"
else
  {
    echo
    echo "## Install timing: \`${FIXTURE_NAME}\` --no-dev"
    echo
    echo "- when: ${DATE_UTC}"
    echo "- host: ${HOST}"
    echo "- composer: \`$( "$COMPOSER_BIN" --version 2>/dev/null | head -1)\`"
    echo
    echo "$TABLE"
    echo
  } >>"$NOTES_TARGET"
  echo "timing: appended to $NOTES_TARGET"
fi
