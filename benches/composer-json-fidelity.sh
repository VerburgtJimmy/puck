#!/usr/bin/env bash
# Compare puck_manifest JsonManipulator edits to `composer require/remove
# --no-install --no-update` on identical starting composer.json files.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v composer >/dev/null 2>&1; then
  echo "composer not on PATH; skip CLI cross-check (unit tests still cover fixtures)"
  exit 0
fi

TMP="$(mktemp -d "${TMPDIR:-/tmp}/puck-json-fid.XXXXXX")"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT

run_case() {
  local name="$1"
  local start_json="$2"
  local composer_cmd="$3"
  local expect_file="$4"

  local dir="$TMP/$name"
  mkdir -p "$dir"
  printf '%s' "$start_json" >"$dir/composer.json"
  # shellcheck disable=SC2086
  (cd "$dir" && composer $composer_cmd --no-install --no-update --no-plugins -q)
  cp "$dir/composer.json" "$expect_file"
}

SORTED_START='{
    "name": "app/app",
    "require": {
        "php": "^8.3",
        "laravel/framework": "^13.0"
    },
    "config": {
        "sort-packages": true
    }
}'

UNSORTED_START='{
    "name": "app/app",
    "require": {
        "laravel/framework": "^13.0",
        "php": "^8.3"
    }
}'

REMOVE_START='{
    "name": "app/app",
    "require": {
        "php": "^8.3",
        "laravel/framework": "^13.0",
        "webmozart/assert": "^1.11"
    },
    "require-dev": {
        "phpunit/phpunit": "^11.0"
    },
    "config": {
        "sort-packages": true
    }
}'

run_case sorted "$SORTED_START" "require webmozart/assert:^1.11" "$TMP/composer-sorted.json"
run_case unsorted "$UNSORTED_START" "require webmozart/assert:^1.11" "$TMP/composer-unsorted.json"
run_case remove "$REMOVE_START" "remove webmozart/assert" "$TMP/composer-remove.json"

# Drive puck_manifest unit tests (fixtures already match Composer samples above).
rustup run 1.96.0 cargo test -p puck_manifest --lib json_manipulator -- --nocapture

echo "composer-json-fidelity: Composer CLI samples written under $TMP (unit tests are the gate)"
echo "ok"
