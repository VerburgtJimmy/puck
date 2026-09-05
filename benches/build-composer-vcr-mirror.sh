#!/usr/bin/env bash
# Build a Composer 2 file:// mirror from fixtures/registry/packagist/p2.
#
# Usage: ./benches/build-composer-vcr-mirror.sh [OUT_DIR]
# Default OUT_DIR: fixtures/registry/composer-mirror
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/fixtures/registry/composer-mirror}"
P2_SRC="$ROOT/fixtures/registry/packagist/p2"

python3 - "$P2_SRC" "$OUT" <<'PY'
import json, sys
from pathlib import Path

src = Path(sys.argv[1])
out = Path(sys.argv[2])
p2_out = out / "p2"
if out.exists():
    # recreate clean
    import shutil
    shutil.rmtree(out)
p2_out.mkdir(parents=True)

packages = []
for path in sorted(src.glob("*.json")):
    # vendor$name.json -> p2/vendor/name.json
    stem = path.stem
    if "$" not in stem:
        continue
    vendor, name = stem.split("$", 1)
    dest_dir = p2_out / vendor
    dest_dir.mkdir(parents=True, exist_ok=True)
    dest = dest_dir / f"{name}.json"
    dest.write_bytes(path.read_bytes())
    packages.append(f"{vendor}/{name}")

meta_url = f"file://{(out / 'p2').resolve()}/%package%.json"
# Packagist sets notify-batch so ComposerRepository injects notification-url
# on every package. Without it, Composer may drop notification-url when
# refreshing packages that lack source (e.g. phpstan/phpstan) on update --lock.
packages_json = {
    "packages": {},
    "metadata-url": meta_url,
    "notify-batch": "https://packagist.org/downloads/",
    "available-packages": packages,
}
(out / "packages.json").write_text(json.dumps(packages_json, indent=2) + "\n")
print(f"mirror={out} packages={len(packages)}")
PY
