# Distribution

Prebuilt binaries are expected on the GitHub release for tag `v0.1.0` (and later). Until assets are attached, `install.sh` and Homebrew will fail to download — that is expected.

## Build release binaries

On each platform (or via CI cross-build):

```bash
cargo build --release -p puck_cli
cp target/release/puck puck-<target-triple>
```

Target triple names used by `install.sh` and `dist/homebrew/puck.rb`:

| Asset name | Platform |
|---|---|
| `puck-aarch64-apple-darwin` | macOS Apple Silicon |
| `puck-x86_64-apple-darwin` | macOS Intel |
| `puck-x86_64-unknown-linux-gnu` | Linux x86_64 |
| `puck-aarch64-unknown-linux-gnu` | Linux arm64 |

## Attach to the GitHub release

```bash
gh release upload v0.1.0 \
  puck-aarch64-apple-darwin \
  puck-x86_64-apple-darwin \
  puck-x86_64-unknown-linux-gnu \
  puck-aarch64-unknown-linux-gnu \
  --clobber
```

Do **not** force-push or retag `v0.1.0` for doc/install commits after the tag; upload assets to the existing release, or cut `v0.1.1` later.

## Homebrew sha256

After uploading assets:

```bash
shasum -a 256 puck-aarch64-apple-darwin
# …update sha256 in dist/homebrew/puck.rb
```

## Install paths

- curl: [`../install.sh`](../install.sh) → `~/.local/bin` or `/usr/local/bin`
- docs: [`../docs/install.md`](../docs/install.md)
- formula: [`../Formula/puck.rb`](../Formula/puck.rb) (Homebrew tap) and copy under [`homebrew/puck.rb`](homebrew/puck.rb)
