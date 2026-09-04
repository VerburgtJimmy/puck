#!/usr/bin/env bash
# M1 parity: compare Composer and puck installs for a fixture.
#
# Usage: ./parity/run.sh <fixture-name> [--no-dev] [--strict]
#
# Default mode is structural (must PASS on laravel-skeleton --no-dev).
# --strict checksums all of vendor/ (expected FAIL until full byte parity).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FIXTURE_NAME=""
NO_DEV=0
STRICT=0

for arg in "$@"; do
  case "$arg" in
    --no-dev) NO_DEV=1 ;;
    --strict) STRICT=1 ;;
    -h|--help)
      echo "usage: $0 <fixture-name> [--no-dev] [--strict]" >&2
      exit 0
      ;;
    *)
      if [[ -z "$FIXTURE_NAME" ]]; then
        FIXTURE_NAME="$arg"
      else
        echo "parity: unexpected argument: $arg" >&2
        exit 2
      fi
      ;;
  esac
done

if [[ -z "$FIXTURE_NAME" ]]; then
  echo "usage: $0 <fixture-name> [--no-dev] [--strict]" >&2
  exit 2
fi

FIXTURE="$ROOT/fixtures/$FIXTURE_NAME"
if [[ ! -f "$FIXTURE/composer.lock" ]]; then
  echo "parity: missing $FIXTURE/composer.lock" >&2
  exit 2
fi

COMPOSER_FLAGS=(install --no-scripts --ignore-platform-reqs --no-interaction --no-ansi)
if [[ "$NO_DEV" -eq 1 ]]; then
  COMPOSER_FLAGS+=(--no-dev)
fi

# Prefer workspace-local temps (sandbox hardlinks to /tmp can fail).
WORKDIR="$(mktemp -d "$ROOT/.tmp-parity.XXXXXX")"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

COMPOSER_DIR="$WORKDIR/composer"
PUCK_DIR="$WORKDIR/puck"
mkdir -p "$COMPOSER_DIR" "$PUCK_DIR"
cp "$FIXTURE/composer.json" "$FIXTURE/composer.lock" "$COMPOSER_DIR/"
cp "$FIXTURE/composer.json" "$FIXTURE/composer.lock" "$PUCK_DIR/"
# Laravel discovery expects bootstrap/ when present in the fixture.
if [[ -d "$FIXTURE/bootstrap" ]]; then
  cp -R "$FIXTURE/bootstrap" "$COMPOSER_DIR/"
  cp -R "$FIXTURE/bootstrap" "$PUCK_DIR/"
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

run_composer_install() {
  local dir="$1"
  local host
  host="$(resolve_composer)"
  if [[ -n "$host" ]]; then
    echo "parity: composer install (host: $host) in $dir"
    (
      cd "$dir"
      "$host" "${COMPOSER_FLAGS[@]}"
    )
    return
  fi
  if command -v docker >/dev/null 2>&1; then
    echo "parity: composer install (docker composer:2) in $dir"
    docker run --rm \
      -v "$dir:/app" \
      -w /app \
      composer:2 \
      "${COMPOSER_FLAGS[@]}"
    return
  fi
  echo "parity: no host composer and no docker" >&2
  exit 2
}

resolve_puck() {
  if [[ -n "${CARGO_BIN:-}" && -x "${CARGO_BIN}" ]]; then
    echo "${CARGO_BIN}"
    return
  fi
  # Prefer cargo run so sandbox/cache target dirs still work.
  echo ""
}

run_puck_install() {
  local dir="$1"
  local bin
  bin="$(resolve_puck)"
  local extra=(--no-scripts)
  if [[ "$NO_DEV" -eq 1 ]]; then
    extra+=(--no-dev)
  fi
  echo "parity: puck install in $dir"
  if [[ -n "$bin" ]]; then
    (
      cd "$dir"
      "$bin" install "${extra[@]}"
    )
  else
    (
      cd "$ROOT"
      rustup run 1.96.0 cargo run -q -p puck_cli -- install --working-dir "$dir" "${extra[@]}"
    )
  fi
}

WARNINGS=()
FAILURES=()
warn() { WARNINGS+=("$1"); echo "parity: WARN $1" >&2; }
fail() { FAILURES+=("$1"); echo "parity: FAIL $1" >&2; }

package_names() {
  local installed="$1"
  php -r '
    $j = json_decode(file_get_contents($argv[1]), true);
    $names = [];
    foreach (($j["packages"] ?? []) as $p) {
      if (!empty($p["name"])) $names[] = strtolower($p["name"]);
    }
    sort($names);
    echo implode("\n", $names);
  ' "$installed"
}

vendor_package_dirs() {
  local root="$1"
  (
    cd "$root"
    if [[ ! -d vendor ]]; then
      exit 0
    fi
    # Package install paths are vendor/<vendor>/<package>.
    find vendor -mindepth 3 -maxdepth 3 -type d \
      ! -path 'vendor/composer/*' \
      ! -path 'vendor/bin/*' \
      | sed 's|^vendor/||' \
      | sort -u
  )
}

payload_checksums() {
  # sha256 of package payload files under vendor/<vendor>/<package>/ (exclude composer + bin + root stubs)
  local root="$1"
  (
    cd "$root"
    if [[ ! -d vendor ]]; then
      exit 0
    fi
    find vendor -mindepth 3 -type f \
      ! -path 'vendor/composer/*' \
      ! -path 'vendor/bin/*' \
      -print0 \
      | sort -z \
      | xargs -0 shasum -a 256 \
      | awk '{print $1 "  " $2}' \
      | sort -k2
  )
}

php_version_keys() {
  local file="$1"
  php -r '
    $v = include $argv[1];
    $keys = array_keys($v["versions"] ?? []);
    sort($keys);
    echo implode("\n", $keys);
  ' "$file"
}

autoload_files_count() {
  local file="$1"
  if [[ ! -f "$file" ]]; then
    echo 0
    return
  fi
  php -r '
    $v = include $argv[1];
    echo is_array($v) ? count($v) : 0;
  ' "$file"
}

autoload_psr4_keys() {
  local file="$1"
  if [[ ! -f "$file" ]]; then
    return
  fi
  php -r '
    $v = include $argv[1];
    $keys = array_keys(is_array($v) ? $v : []);
    sort($keys);
    echo implode("\n", $keys);
  ' "$file"
}

bin_names() {
  local root="$1"
  (
    cd "$root"
    if [[ ! -d vendor/bin ]]; then
      exit 0
    fi
    find vendor/bin -maxdepth 1 -type f -exec basename {} \; | sort
  )
}

checksum_tree() {
  local dir="$1"
  (
    cd "$dir"
    if [[ -d vendor ]]; then
      find vendor -type f -print0 | sort -z | xargs -0 shasum -a 256
    fi
  )
}

run_composer_install "$COMPOSER_DIR"
run_puck_install "$PUCK_DIR"

echo "parity: structural checks for $FIXTURE_NAME"

# 1. Same package names in installed.json
C_NAMES="$(package_names "$COMPOSER_DIR/vendor/composer/installed.json")"
P_NAMES="$(package_names "$PUCK_DIR/vendor/composer/installed.json")"
if [[ "$C_NAMES" != "$P_NAMES" ]]; then
  fail "installed.json package names differ"
  diff -u <(printf '%s\n' "$C_NAMES") <(printf '%s\n' "$P_NAMES") >&2 || true
else
  echo "parity: OK installed.json package names"
fi

# 2. Same vendor package directory set
C_DIRS="$(vendor_package_dirs "$COMPOSER_DIR")"
P_DIRS="$(vendor_package_dirs "$PUCK_DIR")"
if [[ "$C_DIRS" != "$P_DIRS" ]]; then
  fail "vendor package directories differ"
  diff -u <(printf '%s\n' "$C_DIRS") <(printf '%s\n' "$P_DIRS") >&2 || true
else
  echo "parity: OK vendor package directories"
fi

# 3. Package payload files: same relative paths AND same sha256
C_PAY="$(payload_checksums "$COMPOSER_DIR")"
P_PAY="$(payload_checksums "$PUCK_DIR")"
if [[ "$C_PAY" != "$P_PAY" ]]; then
  fail "vendor package payload checksums differ"
  diff -u <(printf '%s\n' "$C_PAY") <(printf '%s\n' "$P_PAY") >&2 || true
else
  echo "parity: OK vendor package payload checksums"
fi

# 4. installed.php version keys identical
if [[ -f "$COMPOSER_DIR/vendor/composer/installed.php" && -f "$PUCK_DIR/vendor/composer/installed.php" ]]; then
  C_VK="$(php_version_keys "$COMPOSER_DIR/vendor/composer/installed.php")"
  P_VK="$(php_version_keys "$PUCK_DIR/vendor/composer/installed.php")"
  if [[ "$C_VK" != "$P_VK" ]]; then
    fail "installed.php version keys differ"
    diff -u <(printf '%s\n' "$C_VK") <(printf '%s\n' "$P_VK") >&2 || true
  else
    echo "parity: OK installed.php version keys"
  fi
else
  fail "installed.php missing on one side"
fi

# 5. autoload_files.php entry count; autoload_psr4.php namespace keys
C_FILES="$(autoload_files_count "$COMPOSER_DIR/vendor/composer/autoload_files.php")"
P_FILES="$(autoload_files_count "$PUCK_DIR/vendor/composer/autoload_files.php")"
if [[ "$C_FILES" != "$P_FILES" ]]; then
  fail "autoload_files.php entry count differs (composer=$C_FILES puck=$P_FILES)"
else
  echo "parity: OK autoload_files.php count ($C_FILES)"
fi

C_PSR4="$(autoload_psr4_keys "$COMPOSER_DIR/vendor/composer/autoload_psr4.php")"
P_PSR4="$(autoload_psr4_keys "$PUCK_DIR/vendor/composer/autoload_psr4.php")"
if [[ "$C_PSR4" != "$P_PSR4" ]]; then
  fail "autoload_psr4.php namespace keys differ"
  diff -u <(printf '%s\n' "$C_PSR4") <(printf '%s\n' "$P_PSR4") >&2 || true
else
  echo "parity: OK autoload_psr4.php namespace keys"
fi

# 6. PHP smoke
smoke_php() {
  local root="$1"
  local with_dev="$2"
  php -r '
    require $argv[1] . "/vendor/autoload.php";
    if (!class_exists("Illuminate\\Foundation\\Application")) {
      fwrite(STDERR, "missing Illuminate\\Foundation\\Application\n");
      exit(1);
    }
    if (!\Composer\InstalledVersions::isInstalled("laravel/framework")) {
      fwrite(STDERR, "InstalledVersions missing laravel/framework\n");
      exit(1);
    }
    if ($argv[2] === "1") {
      if (!class_exists("PHPUnit\\Framework\\TestCase")) {
        fwrite(STDERR, "missing PHPUnit\\Framework\\TestCase\n");
        exit(1);
      }
    }
    echo "ok\n";
  ' "$root" "$with_dev"
}

DEV_FLAG=0
if [[ "$NO_DEV" -eq 0 ]]; then
  DEV_FLAG=1
fi

if ! smoke_php "$COMPOSER_DIR" "$DEV_FLAG" >/dev/null; then
  fail "composer PHP smoke failed"
fi
if ! out="$(smoke_php "$PUCK_DIR" "$DEV_FLAG")"; then
  fail "puck PHP smoke failed"
else
  echo "parity: OK PHP smoke ($out)"
fi

# 7. packages.php when puck wrote it
if [[ -f "$PUCK_DIR/bootstrap/cache/packages.php" ]]; then
  if php -r '
    $v = include $argv[1];
    if (!is_array($v)) { fwrite(STDERR, "not array\n"); exit(1); }
    foreach (["nesbot/carbon", "nunomaduro/termwind", "laravel/tinker"] as $need) {
      // keys may be short names from discovery; accept either form
    }
    $keys = array_keys($v);
    $flat = strtolower(implode(" ", $keys));
    foreach (["carbon", "termwind", "tinker"] as $needle) {
      if (strpos($flat, $needle) === false) {
        fwrite(STDERR, "missing key fragment: $needle\n");
        exit(1);
      }
    }
  ' "$PUCK_DIR/bootstrap/cache/packages.php"; then
    echo "parity: OK bootstrap/cache/packages.php"
  else
    fail "bootstrap/cache/packages.php invalid or missing expected keys"
  fi
elif [[ "$NO_DEV" -eq 1 && "$FIXTURE_NAME" == "laravel-skeleton" ]]; then
  # Discovery may create it; if composer also lacks it, skip.
  if [[ -f "$COMPOSER_DIR/bootstrap/cache/packages.php" ]]; then
    fail "puck missing bootstrap/cache/packages.php (composer has it)"
  else
    warn "packages.php not present on either side"
  fi
fi

# vendor/bin presence must match after bin work
C_BINS="$(bin_names "$COMPOSER_DIR")"
P_BINS="$(bin_names "$PUCK_DIR")"
if [[ "$C_BINS" != "$P_BINS" ]]; then
  fail "vendor/bin names differ"
  diff -u <(printf '%s\n' "$C_BINS") <(printf '%s\n' "$P_BINS") >&2 || true
else
  echo "parity: OK vendor/bin names"
fi

# Allow / warn (do not fail structural)
if [[ -f "$COMPOSER_DIR/vendor/composer/platform_check.php" && ! -f "$PUCK_DIR/vendor/composer/platform_check.php" ]]; then
  warn "platform_check.php present only on composer"
elif [[ ! -f "$COMPOSER_DIR/vendor/composer/platform_check.php" && -f "$PUCK_DIR/vendor/composer/platform_check.php" ]]; then
  warn "platform_check.php present only on puck"
fi

C_CM_COUNT="$(php -r '$v=@include $argv[1]; echo is_array($v)?count($v):0;' "$COMPOSER_DIR/vendor/composer/autoload_classmap.php" 2>/dev/null || echo 0)"
P_CM_COUNT="$(php -r '$v=@include $argv[1]; echo is_array($v)?count($v):0;' "$PUCK_DIR/vendor/composer/autoload_classmap.php" 2>/dev/null || echo 0)"
if [[ "$C_CM_COUNT" != "$P_CM_COUNT" ]]; then
  warn "classmap size differs (composer=$C_CM_COUNT puck=$P_CM_COUNT; Composer may optimize by default)"
fi

# Generated file content differences are warned, not failed
for gen in autoload_real.php autoload_static.php ClassLoader.php InstalledVersions.php installed.json; do
  cf="$COMPOSER_DIR/vendor/composer/$gen"
  pf="$PUCK_DIR/vendor/composer/$gen"
  if [[ -f "$cf" && -f "$pf" ]]; then
    if ! cmp -s "$cf" "$pf"; then
      warn "vendor/composer/$gen content differs (allowed)"
    fi
  fi
done

if [[ "$STRICT" -eq 1 ]]; then
  echo "parity: strict vendor/ checksum compare"
  checksum_tree "$COMPOSER_DIR" >"$WORKDIR/composer.sha"
  checksum_tree "$PUCK_DIR" >"$WORKDIR/puck.sha"
  if ! diff -u "$WORKDIR/composer.sha" "$WORKDIR/puck.sha"; then
    fail "strict: vendor content differs"
  else
    echo "parity: OK strict vendor checksums"
  fi
fi

echo
echo "parity: summary for $FIXTURE_NAME (no_dev=$NO_DEV strict=$STRICT)"
echo "  failures: ${#FAILURES[@]}"
echo "  warnings: ${#WARNINGS[@]}"
if [[ "${#FAILURES[@]}" -gt 0 ]]; then
  for f in "${FAILURES[@]}"; do
    echo "  - $f"
  done
  echo "parity: FAIL $FIXTURE_NAME"
  exit 1
fi

echo "parity: OK $FIXTURE_NAME (structural)"
exit 0
