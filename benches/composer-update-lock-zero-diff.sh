#!/usr/bin/env bash
# M3 gate: puck-written lock must survive `composer update --lock` on VCR metadata.
#
# 1. Build Composer file:// mirror from fixtures/registry/packagist/p2
# 2. Copy fixture into a temp project; point composer.json at the mirror only
# 3. Rewrite lock with puck (fixed pins from existing lock, ArrayDumper from VCR)
# 4. Run `composer update --lock --no-install`
# 5. Assert zero diff on composer.lock
#
# Usage: ./benches/composer-update-lock-zero-diff.sh [laravel-skeleton|laravel-app]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FIXTURE="${1:-laravel-skeleton}"
FIXTURE_DIR="$ROOT/fixtures/$FIXTURE"
MIRROR="$ROOT/fixtures/registry/composer-mirror"
REGISTRY="$ROOT/fixtures/registry"

if [[ ! -f "$FIXTURE_DIR/composer.lock" ]]; then
  echo "missing fixture lock: $FIXTURE_DIR/composer.lock" >&2
  exit 1
fi

"$ROOT/benches/build-composer-vcr-mirror.sh" "$MIRROR"

TMP="$(mktemp -d)"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT

cp "$FIXTURE_DIR/composer.json" "$FIXTURE_DIR/composer.lock" "$TMP/"
MIRROR_URL="file://$MIRROR"

# Inject VCR repo before puck lock so content-hash matches what Composer will see.
python3 - "$TMP/composer.json" "$MIRROR_URL" <<'PY'
import json, sys
from pathlib import Path
path = Path(sys.argv[1])
url = sys.argv[2]
data = json.loads(path.read_text())
data["repositories"] = [
    {"type": "composer", "url": url},
    {"packagist": False},
]
path.write_text(json.dumps(data, indent=4) + "\n")
PY

cd "$ROOT"
rustup run 1.96.0 cargo run -p puck_cli --quiet -- lock \
  --no-install \
  --working-dir "$TMP" \
  --registry "$REGISTRY"

cp "$TMP/composer.lock" "$TMP/composer.lock.puck"
cd "$TMP"
# Offline: only the file:// mirror.
composer update --lock --no-install --no-audit --no-scripts --no-ansi 2>"$TMP/composer.err" || {
  echo "composer update --lock failed:" >&2
  cat "$TMP/composer.err" >&2
  exit 1
}
if ! diff -u composer.lock.puck composer.lock; then
  echo "FAIL: composer update --lock rewrote the puck lock ($FIXTURE)" >&2
  exit 1
fi
echo "PASS: composer update --lock zero diff ($FIXTURE)"
