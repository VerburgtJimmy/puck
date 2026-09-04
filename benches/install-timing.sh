#!/usr/bin/env bash
# Cold/warm install timings: Composer vs puck on a fixture.
#
# Usage: ./benches/install-timing.sh [fixture-name]
# Default fixture: laravel-skeleton (always --no-dev).
#
# Prints a markdown table to stdout and appends/updates
# ~/Developer/personal/puck-notes/docs/benchmarks.md
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
    rustup run 1.96.0 cargo run -q -p puck_cli -- install --working-dir "$dir" --no-dev
  )
}

run_composer() {
  local dir="$1"
  (
    cd "$dir"
    "$COMPOSER_BIN" install --no-dev --no-scripts --ignore-platform-reqs --no-interaction --no-ansi
  )
}

echo "timing: fixture=$FIXTURE_NAME (--no-dev)"
echo "timing: composer=$COMPOSER_BIN"
echo "timing: warming puck binary (cargo run once)…"
# Warm the puck binary compile so cold install timings exclude rustc.
DUMMY="$WORKDIR/warm-compile"
prepare_tree "$DUMMY"
# Ensure lock-only tree; install may fail offline briefly — still builds binary.
rustup run 1.96.0 cargo build -q -p puck_cli

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

mkdir -p "$(dirname "$NOTES_DOC")"
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
} >>"$NOTES_DOC"

echo "timing: appended to $NOTES_DOC"
