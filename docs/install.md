# Install puck

**Keep Composer installed; puck reads and writes the same files and you can switch back at any time.**

puck targets macOS and Linux (amd64 / arm64). Linux releases are **musl** static binaries. Windows is not supported in 0.1.

See [`distribution.md`](distribution.md) for release artifacts, signing, and upgrade policy.

## curl

```bash
curl -fsSL https://raw.githubusercontent.com/VerburgtJimmy/puck/v0.1.2/install.sh | bash
```

Pinned to the release tag so the script matches that release. Site mirror:
`https://puck.jimmyverburgt.com/install` - never the source of truth for upgrades;
the GitHub Releases `manifest.json` is. Resolving `latest` without pinning
`PUCK_VERSION` requires `python3` on PATH (to parse the manifest).

Installs into `~/.puck/bin` and appends a PATH block to your shell rc unless `--no-modify-path`.

```bash
# pin a different binary version (script still from the tag above, or swap the tag)
curl -fsSL https://raw.githubusercontent.com/VerburgtJimmy/puck/v0.1.2/install.sh | bash -s -- v0.1.2

# uninstall binary + PATH block (store left behind)
curl -fsSL https://raw.githubusercontent.com/VerburgtJimmy/puck/v0.1.2/install.sh | bash -s -- --uninstall
```

| Variable / flag | Default | Meaning |
|---|---|---|
| `PUCK_INSTALL` | `~/.puck` | Install root (`bin/puck` underneath) |
| `PUCK_VERSION` / first arg | latest via manifest | GitHub release tag |
| `PUCK_REPO` | `VerburgtJimmy/puck` | GitHub repo |
| `--no-modify-path` | off | Do not edit shell rc |
| `-v` | off | Verbose |

Always verifies SHA256. Verifies minisign when the `minisign` tool is installed. Public key: [`../dist/minisign/minisign.pub`](../dist/minisign/minisign.pub).

If Homebrew’s `puck` is already on `PATH`, the script refuses and tells you to `brew upgrade puck`.

## Homebrew

```bash
brew tap VerburgtJimmy/puck https://github.com/VerburgtJimmy/puck
brew install puck
```

Live formula: [`../Formula/puck.rb`](../Formula/puck.rb), regenerated from each release `SHA256SUMS` by the release workflow (`dist/homebrew/generate-formula.rb`). [`../dist/homebrew/puck.rb`](../dist/homebrew/puck.rb) remains a template only.

## From source

```bash
git clone https://github.com/VerburgtJimmy/puck.git
cd puck
cargo build --release -p puck_cli
mkdir -p ~/.puck/bin
install -m 755 target/release/puck ~/.puck/bin/puck
```

Requires Rust 1.96+.

## After install

```bash
puck doctor          # check the current project
puck install         # same lock/vendor files as Composer
```

## GitHub Actions

```yaml
- uses: VerburgtJimmy/puck/.github/actions/setup-puck@v0.1.2
  with:
    version: latest
    cache: true
```

The action runs the `install.sh` from the same tagged checkout (not a live
`master` fetch). See [`distribution.md`](distribution.md) for Docker and upgrade policy.

## Upgrade

```bash
puck upgrade              # latest stable from manifest.json
puck upgrade --version 0.1.0
puck upgrade --rollback   # restore ~/.puck/bin/puck.previous
```

Homebrew installs must use `brew upgrade puck` - `puck upgrade` refuses Cellar paths.
