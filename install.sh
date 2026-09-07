#!/usr/bin/env bash
# Install puck into ~/.puck (see docs/distribution.md).
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/VerburgtJimmy/puck/master/install.sh | bash
# Mirror (when ready): https://puck.jimmyverburgt.com/install
#
# Options: --uninstall  --no-modify-path  -v  [VERSION]
# Env: PUCK_VERSION  PUCK_INSTALL  PUCK_REPO

if [ -z "${BASH_VERSION:-}" ]; then
  echo "error: install.sh requires bash (not sh/dash). Re-run with bash or: curl … | bash" >&2
  exit 1
fi

set -euo pipefail

# --- constants (keep in sync with docs/distribution.md / crates/puck_cli/src/dist_urls.rs) ---
MANIFEST_URL="https://github.com/VerburgtJimmy/puck/releases/latest/download/manifest.json"
INSTALL_MIRROR="https://puck.jimmyverburgt.com/install"
STABLE_MIRROR="https://puck.jimmyverburgt.com/releases/stable.json"
REPO="${PUCK_REPO:-VerburgtJimmy/puck}"
DEFAULT_INSTALL_ROOT="${HOME}/.puck"
MINISIGN_PUBKEY="RWRNlu02PTyL13V6QL9fxE4Ho6fcGHI/5fu6HdGzQ1mlKghwOWDg/6ft"

VERBOSE=0
MODIFY_PATH=1
UNINSTALL=0
VERSION_ARG=""

log() { printf '%s\n' "$*"; }
vlog() { if [[ "$VERBOSE" -eq 1 ]]; then printf 'verbose: %s\n' "$*" >&2; fi; }
err() { printf 'error: %s\n' "$*" >&2; }

usage() {
  cat <<'USAGE'
Usage: install.sh [VERSION] [--uninstall] [--no-modify-path] [-v]

Installs puck into ~/.puck/bin (override with PUCK_INSTALL).
VERSION may also be set via PUCK_VERSION (e.g. v0.1.0). Without a version,
fetches the latest release manifest from GitHub Releases.

  --uninstall       Remove ~/.puck/bin/puck and the PATH block from shell rc
  --no-modify-path  Do not append PATH to shell rc
  -v                Verbose
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --uninstall) UNINSTALL=1 ;;
    --no-modify-path) MODIFY_PATH=0 ;;
    -v|--verbose) VERBOSE=1 ;;
    -h|--help) usage; exit 0 ;;
    -*)
      err "unknown option: $1"
      usage >&2
      exit 1
      ;;
    *)
      if [[ -n "$VERSION_ARG" ]]; then
        err "unexpected argument: $1"
        exit 1
      fi
      VERSION_ARG="$1"
      ;;
  esac
  shift
done

PUCK_INSTALL="${PUCK_INSTALL:-$DEFAULT_INSTALL_ROOT}"
BIN_DIR="${PUCK_INSTALL}/bin"
BIN_PATH="${BIN_DIR}/puck"
PATH_MARKER_BEGIN="# puck PATH begin"
PATH_MARKER_END="# puck PATH end"

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    err "required command not found: $1"
    exit 1
  fi
}

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin) os="apple-darwin" ;;
    Linux) os="unknown-linux-musl" ;;
    *)
      err "unsupported OS '$os'"
      err "supported: aarch64/x86_64 macOS (apple-darwin) and Linux (unknown-linux-musl)"
      exit 1
      ;;
  esac
  case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *)
      err "unsupported architecture '$arch'"
      err "supported: aarch64, x86_64"
      exit 1
      ;;
  esac
  # Prefer native arm64 over Rosetta.
  if [[ "$(uname -s)" == "Darwin" && "$arch" == "x86_64" ]] && [[ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" == "1" ]]; then
    arch="aarch64"
    vlog "detected Rosetta; using aarch64-apple-darwin"
  fi
  echo "${arch}-${os}"
}

detect_shell_rc() {
  local shell_name
  shell_name="$(basename "${SHELL:-}")"
  case "$shell_name" in
    zsh) echo "${ZDOTDIR:-$HOME}/.zshrc" ;;
    bash)
      if [[ -f "${HOME}/.bash_profile" ]]; then
        echo "${HOME}/.bash_profile"
      else
        echo "${HOME}/.bashrc"
      fi
      ;;
    *) echo "${HOME}/.profile" ;;
  esac
}

path_block() {
  cat <<BLOCK
${PATH_MARKER_BEGIN}
export PATH="${BIN_DIR}:\$PATH"
${PATH_MARKER_END}
BLOCK
}

remove_path_block() {
  local rc="$1"
  [[ -f "$rc" ]] || return 0
  local tmp
  tmp="$(mktemp)"
  awk -v b="$PATH_MARKER_BEGIN" -v e="$PATH_MARKER_END" '
    $0 == b {skip=1; next}
    $0 == e {skip=0; next}
    !skip {print}
  ' "$rc" > "$tmp"
  mv "$tmp" "$rc"
}

append_path_block() {
  local rc="$1"
  touch "$rc"
  if grep -Fq "$PATH_MARKER_BEGIN" "$rc" 2>/dev/null; then
    vlog "PATH block already present in $rc"
    return 0
  fi
  {
    printf '\n'
    path_block
  } >> "$rc"
  log "added PATH block to ${rc}:"
  log "  export PATH=\"${BIN_DIR}:\$PATH\""
  log "remove later with: install.sh --uninstall  (or delete the puck PATH begin/end block)"
}

refuse_homebrew_conflict() {
  local which_puck=""
  which_puck="$(command -v puck 2>/dev/null || true)"
  if [[ -n "$which_puck" ]]; then
    case "$which_puck" in
      */Cellar/puck/*|*/opt/puck/*)
        err "Homebrew puck is on PATH ($which_puck)"
        err "use: brew upgrade puck"
        exit 1
        ;;
    esac
  fi
}

do_uninstall() {
  log "uninstalling puck from ${BIN_PATH}"
  rm -f "$BIN_PATH" "${BIN_PATH}.previous" 2>/dev/null || true
  local rc
  rc="$(detect_shell_rc)"
  remove_path_block "$rc"
  log "removed binary and PATH block (if present)"
  if [[ -d "${PUCK_INSTALL}/store" ]]; then
    log "note: package store left behind under ${PUCK_INSTALL}; delete manually if desired"
  fi
  exit 0
}

sha256_file() {
  local f="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$f" | awk '{print $1}'
  else
    shasum -a 256 "$f" | awk '{print $1}'
  fi
}

verify_sha256() {
  local archive="$1"
  local sums="$2"
  local base expected actual
  base="$(basename "$archive")"
  expected="$(awk -v b="$base" '$2 == b || $2 == ("*" b) {print $1; exit}' "$sums")"
  if [[ -z "$expected" ]]; then
    err "no sha256 entry for $base in SHA256SUMS"
    exit 1
  fi
  actual="$(sha256_file "$archive")"
  if [[ "$expected" != "$actual" ]]; then
    err "sha256 mismatch for $base"
    err "  expected: $expected"
    err "  actual:   $actual"
    rm -f "$archive"
    exit 1
  fi
  vlog "sha256 ok for $base"
}

verify_minisign_optional() {
  local sums="$1"
  local sig="$2"
  if ! command -v minisign >/dev/null 2>&1; then
    vlog "minisign not installed; skipping signature verify"
    return 0
  fi
  if [[ ! -f "$sig" ]]; then
    err "SHA256SUMS.minisig missing; cannot verify with minisign"
    exit 1
  fi
  local pubtmp
  pubtmp="$(mktemp)"
  printf 'untrusted comment: puck install minisign public key\n%s\n' "$MINISIGN_PUBKEY" > "$pubtmp"
  if ! minisign -Vm "$sums" -x "$sig" -p "$pubtmp"; then
    rm -f "$pubtmp"
    err "minisign verification failed"
    exit 1
  fi
  rm -f "$pubtmp"
  vlog "minisign ok"
}

fetch() {
  local url="$1"
  local out="$2"
  vlog "GET $url"
  if ! curl -fsSL "$url" -o "$out"; then
    err "failed to download $url"
    exit 1
  fi
}

resolve_version_and_url() {
  local target="$1"
  VERSION="${VERSION_ARG:-${PUCK_VERSION:-}}"
  EXPECTED_SHA=""
  ASSET_URL=""

  if [[ -z "$VERSION" ]]; then
    need_cmd curl
    local manifest
    manifest="$(mktemp)"
    vlog "fetching manifest $MANIFEST_URL"
    if ! curl -fsSL "$MANIFEST_URL" -o "$manifest"; then
      err "failed to fetch $MANIFEST_URL"
      err "optional mirror $STABLE_MIRROR is never the source of truth"
      rm -f "$manifest"
      exit 1
    fi
    if ! command -v python3 >/dev/null 2>&1; then
      err "python3 required to parse manifest.json (or set PUCK_VERSION)"
      rm -f "$manifest"
      exit 1
    fi
    local parsed
    parsed="$(python3 -c '
import json, sys
m = json.load(open(sys.argv[1]))
target = sys.argv[2]
ver = str(m.get("version", ""))
tag = ver if ver.startswith("v") else ("v" + ver if ver else "")
art = (m.get("artifacts") or {}).get(target) or {}
url = art.get("url") or ""
sha = art.get("sha256") or ""
print(tag)
print(url)
print(sha)
' "$manifest" "$target")"
    VERSION="$(printf '%s\n' "$parsed" | sed -n '1p')"
    ASSET_URL="$(printf '%s\n' "$parsed" | sed -n '2p')"
    EXPECTED_SHA="$(printf '%s\n' "$parsed" | sed -n '3p')"
    rm -f "$manifest"
  fi

  if [[ -z "$VERSION" ]]; then
    err "could not resolve version"
    exit 1
  fi
  if [[ "$VERSION" != v* ]]; then
    VERSION="v${VERSION}"
  fi

  if [[ -z "$ASSET_URL" ]]; then
    ASSET_URL="https://github.com/${REPO}/releases/download/${VERSION}/puck-${target}.tar.gz"
  fi
}

do_install() {
  need_cmd curl
  need_cmd tar
  refuse_homebrew_conflict

  local target
  target="$(detect_target)"
  resolve_version_and_url "$target"

  local asset="puck-${target}.tar.gz"
  local base_url="https://github.com/${REPO}/releases/download/${VERSION}"
  local tmp
  tmp="$(mktemp -d)"
  # Expand path now: a local `tmp` is gone by EXIT, and `set -u` would fail.
  # shellcheck disable=SC2064
  trap "rm -rf $(printf '%q' "$tmp")" EXIT

  log "puck install: ${VERSION} (${target}) -> ${BIN_PATH}"

  fetch "${ASSET_URL}" "${tmp}/${asset}"
  fetch "${base_url}/SHA256SUMS" "${tmp}/SHA256SUMS"
  if curl -fsSL "${base_url}/SHA256SUMS.minisig" -o "${tmp}/SHA256SUMS.minisig" 2>/dev/null; then
    verify_minisign_optional "${tmp}/SHA256SUMS" "${tmp}/SHA256SUMS.minisig"
  else
    vlog "no SHA256SUMS.minisig at release; sha256-only verify"
  fi
  verify_sha256 "${tmp}/${asset}" "${tmp}/SHA256SUMS"

  if [[ -n "$EXPECTED_SHA" ]]; then
    local actual
    actual="$(sha256_file "${tmp}/${asset}")"
    if [[ "$EXPECTED_SHA" != "$actual" ]]; then
      err "manifest sha256 mismatch for $asset"
      err "  expected: $EXPECTED_SHA"
      err "  actual:   $actual"
      exit 1
    fi
  fi

  mkdir -p "$BIN_DIR"
  local extract
  extract="$(mktemp -d "${tmp}/extract.XXXXXX")"
  tar -xzf "${tmp}/${asset}" -C "$extract"
  local built
  built="$(find "$extract" -type f -name puck | head -n 1)"
  if [[ -z "$built" || ! -f "$built" ]]; then
    err "archive did not contain a puck binary"
    exit 1
  fi
  chmod 755 "$built"
  local stage="${BIN_PATH}.new"
  mv "$built" "$stage"
  if [[ -f "$BIN_PATH" ]]; then
    cp -f "$BIN_PATH" "${BIN_PATH}.previous" 2>/dev/null || true
  fi
  mv -f "$stage" "$BIN_PATH"

  if [[ "$MODIFY_PATH" -eq 1 ]]; then
    local rc
    rc="$(detect_shell_rc)"
    case ":${PATH}:" in
      *":${BIN_DIR}:"*) vlog "${BIN_DIR} already on PATH" ;;
      *) append_path_block "$rc" ;;
    esac
  else
    log "skipped PATH modification (--no-modify-path)"
    log "add to PATH: export PATH=\"${BIN_DIR}:\$PATH\""
  fi

  local ver_out
  ver_out="$("$BIN_PATH" --version 2>/dev/null || true)"
  log "installed: ${BIN_PATH}"
  [[ -n "$ver_out" ]] && log "version:   ${ver_out}"
  log "next:      puck doctor"
  if ! command -v puck >/dev/null 2>&1; then
    log "note: open a new shell or: export PATH=\"${BIN_DIR}:\$PATH\""
  fi
  : "$INSTALL_MIRROR" "$STABLE_MIRROR"
}

if [[ "$UNINSTALL" -eq 1 ]]; then
  do_uninstall
fi
do_install
