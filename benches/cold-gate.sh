#!/usr/bin/env bash
# Fail if cold puck loses to Composer, or warm-wipe / warm-keep regress vs Composer.
# Reads GATE lines from install-timing logs (stdin or files as args).
set -euo pipefail

fail=0
check_gate() {
  local line="$1"
  [[ "$line" == GATE* ]] || return 0
  # shellcheck disable=SC2086
  eval "${line#GATE }"
  echo "gate: fixture=${fixture:-?} cold composer=${cold_composer_ms} puck=${cold_puck_ms} wipe ${warm_wipe_composer_ms}/${warm_wipe_puck_ms} keep ${warm_keep_composer_ms}/${warm_keep_puck_ms}"
  if [[ "${cold_puck_ms}" -gt "${cold_composer_ms}" ]]; then
    echo "gate FAIL: cold puck (${cold_puck_ms} ms) > composer (${cold_composer_ms} ms)" >&2
    fail=1
  fi
  if [[ "${warm_wipe_puck_ms}" -gt "${warm_wipe_composer_ms}" ]]; then
    echo "gate FAIL: warm-wipe puck (${warm_wipe_puck_ms} ms) > composer (${warm_wipe_composer_ms} ms)" >&2
    fail=1
  fi
  if [[ "${warm_keep_puck_ms}" -gt "${warm_keep_composer_ms}" ]]; then
    echo "gate FAIL: warm-keep puck (${warm_keep_puck_ms} ms) > composer (${warm_keep_composer_ms} ms)" >&2
    fail=1
  fi
}

if [[ $# -eq 0 ]]; then
  while IFS= read -r line; do
    check_gate "$line"
  done
else
  for f in "$@"; do
    while IFS= read -r line; do
      check_gate "$line"
    done <"$f"
  done
fi

if [[ "$fail" -ne 0 ]]; then
  echo "gate: FAILED — report PUCK_TIMINGS phase lines and stop; do not tune further without a root cause" >&2
  exit 1
fi
echo "gate: OK"
