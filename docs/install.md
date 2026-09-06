# Install puck

**Keep Composer installed; puck reads and writes the same files and you can switch back at any time.**

puck targets macOS and Linux (amd64 / arm64). Windows is not supported in 0.1.

## curl

```bash
curl -fsSL https://raw.githubusercontent.com/VerburgtJimmy/puck/master/install.sh | bash
```

Options (env):

| Variable | Default | Meaning |
|---|---|---|
| `PUCK_INSTALL_DIR` | `~/.local/bin` (or `/usr/local/bin` if writable) | Install location |
| `PUCK_VERSION` | `v0.1.0` | GitHub release tag |
| `PUCK_REPO` | `VerburgtJimmy/puck` | GitHub repo |

Ensure the install directory is on your `PATH`.

The script downloads a prebuilt binary from the matching GitHub release. Release assets must be uploaded for the tag you select — see [`../dist/README.md`](../dist/README.md).

## Homebrew

```bash
brew tap VerburgtJimmy/puck https://github.com/VerburgtJimmy/puck
brew install puck
```

The formula lives at [`../dist/homebrew/puck.rb`](../dist/homebrew/puck.rb). Homebrew expects release tarballs/binaries attached to the GitHub release.

## From source

```bash
git clone https://github.com/VerburgtJimmy/puck.git
cd puck
cargo build --release -p puck_cli
install -m 755 target/release/puck ~/.local/bin/puck
```

Requires Rust 1.96+.

## After install

```bash
puck doctor          # check the current project
puck install         # same lock/vendor files as Composer
```
