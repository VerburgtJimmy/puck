#!/usr/bin/env bash
# Compare Composer and puck installs for a fixture.
#
# Usage: ./parity/run.sh <fixture-name>
# Expects: fixtures/<name>/ with composer.json + composer.lock
# Requires: docker (Composer), a built `puck` on PATH or via CARGO_BIN
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FIXTURE_NAME="${1:-}"

if [[ -z "$FIXTURE_NAME" ]]; then
  echo "usage: $0 <fixture-name>" >&2
  exit 2
fi

FIXTURE="$ROOT/fixtures/$FIXTURE_NAME"
if [[ ! -f "$FIXTURE/composer.lock" ]]; then
  echo "parity: missing $FIXTURE/composer.lock" >&2
  exit 2
fi

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/puck-parity.XXXXXX")"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

COMPOSER_DIR="$WORKDIR/composer"
PUCK_DIR="$WORKDIR/puck"
mkdir -p "$COMPOSER_DIR" "$PUCK_DIR"
cp "$FIXTURE/composer.json" "$FIXTURE/composer.lock" "$COMPOSER_DIR/"
cp "$FIXTURE/composer.json" "$FIXTURE/composer.lock" "$PUCK_DIR/"

echo "parity: composer install (docker) in $COMPOSER_DIR"
docker run --rm \
  -v "$COMPOSER_DIR:/app" \
  -w /app \
  composer:2 \
  install --no-interaction --no-ansi --ignore-platform-reqs

PUCK_BIN="${CARGO_BIN:-$ROOT/target/debug/puck}"
if [[ ! -x "$PUCK_BIN" ]]; then
  echo "parity: building puck"
  cargo build -p puck_cli
  PUCK_BIN="$ROOT/target/debug/puck"
fi

echo "parity: puck install in $PUCK_DIR"
(
  cd "$PUCK_DIR"
  "$PUCK_BIN" install --offline || "$PUCK_BIN" install
)

echo "parity: diffing vendor/ (excluding mtimes via content)"
# Content-oriented compare: checksums of all files under vendor/
checksum_tree() {
  local dir="$1"
  (
    cd "$dir"
    if [[ -d vendor ]]; then
      find vendor -type f -print0 | sort -z | xargs -0 shasum -a 256
    fi
  )
}

checksum_tree "$COMPOSER_DIR" >"$WORKDIR/composer.sha"
checksum_tree "$PUCK_DIR" >"$WORKDIR/puck.sha"

if ! diff -u "$WORKDIR/composer.sha" "$WORKDIR/puck.sha"; then
  echo "parity: FAIL vendor content differs for $FIXTURE_NAME" >&2
  exit 1
fi

if [[ -f "$COMPOSER_DIR/bootstrap/cache/packages.php" || -f "$PUCK_DIR/bootstrap/cache/packages.php" ]]; then
  diff -u \
    "$COMPOSER_DIR/bootstrap/cache/packages.php" \
    "$PUCK_DIR/bootstrap/cache/packages.php"
fi

echo "parity: OK $FIXTURE_NAME"
