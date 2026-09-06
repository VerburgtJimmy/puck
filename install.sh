#!/usr/bin/env bash
# Install puck from GitHub Releases.
# Usage: curl -fsSL https://raw.githubusercontent.com/VerburgtJimmy/puck/master/install.sh | bash
set -euo pipefail

REPO="${PUCK_REPO:-VerburgtJimmy/puck}"
VERSION="${PUCK_VERSION:-v0.1.0}"

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin) os="apple-darwin" ;;
    Linux) os="unknown-linux-gnu" ;;
    *)
      echo "error: unsupported OS '$os' (macOS and Linux only)" >&2
      exit 1
      ;;
  esac
  case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *)
      echo "error: unsupported architecture '$arch'" >&2
      exit 1
      ;;
  esac
  echo "${arch}-${os}"
}

default_install_dir() {
  if [[ -n "${PUCK_INSTALL_DIR:-}" ]]; then
    echo "$PUCK_INSTALL_DIR"
    return
  fi
  if [[ -w /usr/local/bin ]] || [[ -w /usr/local/bin 2>/dev/null ]]; then
    if [[ -d /usr/local/bin ]] && [[ -w /usr/local/bin ]]; then
      echo "/usr/local/bin"
      return
    fi
  fi
  echo "${HOME}/.local/bin"
}

TARGET="$(detect_target)"
ASSET="puck-${TARGET}"
INSTALL_DIR="$(default_install_dir)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}"

echo "puck install: ${VERSION} (${TARGET}) -> ${INSTALL_DIR}/puck"
echo "  fetching ${URL}"

if ! curl -fsSL "$URL" -o "${TMP}/puck"; then
  cat >&2 <<MSG
error: failed to download ${URL}

Release assets may not be uploaded yet. Attach platform binaries to the
GitHub release (see dist/README.md), or build from source:

  cargo build --release -p puck_cli
  install -m 755 target/release/puck ${INSTALL_DIR}/puck
MSG
  exit 1
fi

mkdir -p "$INSTALL_DIR"
chmod 755 "${TMP}/puck"
mv "${TMP}/puck" "${INSTALL_DIR}/puck"

echo "installed: ${INSTALL_DIR}/puck"
if ! command -v puck >/dev/null 2>&1; then
  echo "note: add ${INSTALL_DIR} to your PATH" >&2
fi
"${INSTALL_DIR}/puck" --version 2>/dev/null || true
