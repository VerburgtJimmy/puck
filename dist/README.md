# Distribution

Source of truth: [`../docs/distribution.md`](../docs/distribution.md).

Prebuilt binaries are produced by [`.github/workflows/release.yml`](../.github/workflows/release.yml) on `v*` tags — never from a laptop. Re-tag from the workflow; do not attach hand-built binaries.

## Artifacts (per tag)

| Asset | Platform |
|---|---|
| `puck-aarch64-apple-darwin.tar.gz` | macOS Apple Silicon (ad-hoc codesign in 0.1) |
| `puck-x86_64-apple-darwin.tar.gz` | macOS Intel (ad-hoc codesign in 0.1) |
| `puck-x86_64-unknown-linux-musl.tar.gz` | Linux x86_64 static |
| `puck-aarch64-unknown-linux-musl.tar.gz` | Linux arm64 static |
| `SHA256SUMS` / `SHA256SUMS.minisig` | checksums + minisign |
| `manifest.json` | what `install.sh` / `puck upgrade` read |

Do **not** ship glibc Linux builds. Manifest URL:

`https://github.com/VerburgtJimmy/puck/releases/latest/download/manifest.json`

## minisign

Public key: [`minisign/minisign.pub`](minisign/minisign.pub). See [`minisign/README.md`](minisign/README.md).

## Install paths

- curl: [`../install.sh`](../install.sh) → `~/.puck/bin`
- docs: [`../docs/install.md`](../docs/install.md)
- Homebrew formula: **template only** — [`homebrew/puck.rb`](homebrew/puck.rb) and [`../Formula/puck.rb`](../Formula/puck.rb)
