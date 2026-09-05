#!/usr/bin/env bash
# Record Packagist Composer 2 metadata for packages in fixture lock files.
#
# Usage: ./benches/record-packagist-p2.sh
# Writes fixtures/registry/packagist/p2/vendor$name.json and SOURCE.json
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
python3 <<'PY'
import json, time, urllib.request
from pathlib import Path

root = Path("fixtures/registry/packagist/p2")
root.mkdir(parents=True, exist_ok=True)

def names_from(lock_path: Path):
    lock = json.loads(lock_path.read_text())
    out = set()
    for p in lock.get("packages", []) + lock.get("packages-dev", []):
        out.add(p["name"].lower())
    return out

names = set()
for lock in Path("fixtures").glob("*/composer.lock"):
    names |= names_from(lock)

meta = {
    "recorded_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "source": "https://repo.packagist.org",
    "api": "Composer 2 /p2/{vendor}/{package}.json",
    "packages": sorted(names),
}
Path("fixtures/registry/SOURCE.json").write_text(json.dumps(meta, indent=2) + "\n")

ok = fail = 0
for name in sorted(names):
    url = f"https://repo.packagist.org/p2/{name}.json"
    dest = root / f"{name.replace('/', '$')}.json"
    try:
        with urllib.request.urlopen(url, timeout=60) as r:
            dest.write_bytes(r.read())
        ok += 1
    except Exception as e:
        fail += 1
        print(f"FAIL {name}: {e}")

print(f"recorded={ok} failed={fail} total={len(names)}")
raise SystemExit(1 if fail else 0)
PY
