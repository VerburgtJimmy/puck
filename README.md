# puck — native tooling for PHP and Laravel, starting with the install.

Composer-compatible installer for PHP. Faster on CI, drop-in with a preflight.

Reads the same `composer.json` / `composer.lock` as Composer, writes a compatible `vendor/` tree, and aims for byte-identical output (autoload files, `installed.json`, `installed.php`) without running PHP on the install path.

**Keep Composer installed;** puck reads and writes the same files and you can switch back at any time.

## Status

**v0.1.1** — macOS and Linux. `install`, `require`, `remove`, `update` / `lock`, Tier 1 adapters for Pest and phpstan/extension-installer, `puck doctor` preflight. Run `puck doctor` before switching a project.

## Benchmarks (Linux CI)

Lead with **warm-wipe** (vendor wiped, cache/store warm) — the honest CI number. **Warm-keep** is a no-op when nothing changed. **Cold** is included for honesty (empty vendor); it is network-heavy and not the headline.

Same Linux CI run ([34119187733](https://github.com/VerburgtJimmy/puck/actions/runs/34119187733)). Scripts: [`benches/`](benches/).

### laravel-skeleton `--no-dev`

| scenario | composer (ms) | puck (ms) |
|---|---:|---:|
| warm (wipe vendor, cache/store warm) | 1736 | **344** |
| warm (vendor present) | 914 | **24** |
| cold (empty vendor) | 6662 | **1850** |

### laravel-app with require-dev

| scenario | composer (ms) | puck (ms) |
|---|---:|---:|
| warm (wipe vendor, cache/store warm) | 2843 | **497** |
| warm (vendor present) | 1428 | **26** |
| cold (empty vendor) | 4311 | **1547** |

Warm-wipe on laravel-app with-dev is dominated by the optimized classmap dump, which scales with require-dev. Warm-keep stays ~O(1) in package count (lock hash + `installed.json` + `vendor/` check).

Cold installs pipeline download (default concurrency 12) with extract (`min(CPUs, 8)` workers) and link packages as they land.
## Compatibility

What blocks switching today (same checks as `puck doctor`):

| Blocker | Notes |
|---|---|
| Unsupported allowed composer-plugin | No silent skip; Tier 1 adapters cover Pest + phpstan/extension-installer only |
| `vcs` / `artifact` / `package` repositories | Composer + path repos only in 0.1 |
| Windows | macOS and Linux only |
| Unreproducible lock | content-hash mismatch or unsupported `plugin-api-version` |

Warnings (abandoned packages, etc.) do not block. Run `puck doctor` or `puck doctor --json` on your project.

## Install

**Keep Composer installed; puck reads and writes the same files and you can switch back at any time.**

### curl

```bash
curl -fsSL https://raw.githubusercontent.com/VerburgtJimmy/puck/v0.1.1/install.sh | bash
```

Pinned to the `v0.1.1` tag so the script matches that release. Mirror (when ready): `https://puck.jimmyverburgt.com/install`. Upgrade/manifest source of truth remains GitHub Releases — see [`docs/distribution.md`](docs/distribution.md) and [`docs/install.md`](docs/install.md).

Installs into `~/.puck/bin`. minisign public key: [`dist/minisign/minisign.pub`](dist/minisign/minisign.pub).

### Homebrew

```bash
brew tap VerburgtJimmy/puck https://github.com/VerburgtJimmy/puck
brew install puck
```

Formula: [`Formula/puck.rb`](Formula/puck.rb) (regenerated from each release `SHA256SUMS`).

### From source

```bash
git clone https://github.com/VerburgtJimmy/puck.git
cd puck
cargo build --release -p puck_cli
# binary: target/release/puck
```

Requires Rust 1.96+.

## Usage

```bash
puck doctor                  # blockers before switching
puck install                 # install from composer.lock
puck install --no-dev        # production install
puck install --offline       # warm store only; no network
puck dump-autoload -o        # regenerate optimized autoload
puck require vendor/package
puck remove vendor/package
puck update                  # refresh lock + install
puck upgrade                 # self-update from GitHub Releases
puck store path              # print the global store path
puck store gc                # garbage-collect unused store entries
```

Set `PUCK_TIMINGS=1` to print phase timings on stderr (`plan_ms`, `download_ms`, `extract_ms`, `overlap_ms`, `link_ms`, `dump_ms`, …).

## Roadmap

puck grows in layers. Each layer is usable on its own, and nothing above a layer ships until the layer below is trusted.

| Layer | Status |
|---|---|
| Packages | 0.1 shipped |
| PHP | Planned |
| Processes | Planned |
| Runtime | Planned |
| Artifacts | Planned |

The runtime layer embeds the official PHP engine directly, the way FrankenPHP does, with puck's own server and worker model around it. puck will not replace the engine itself.

**0.1 (now):** install path, Composer + path repos with auth, doctor preflight, Pest + phpstan Tier 1, Tier 3 refusal for other plugins.

**0.2:** VCS / artifact / package repositories, filter-list / audit polish, more Tier 1 adapters, canary channel after parity stays green on `master`, Apple notarization.

## How it differs

| | Composer | puck |
|---|---|---|
| Runtime for install | PHP | Native (Rust) |
| Package cache | Per-project / Composer cache | Shared content-addressable store (`~/.puck/store`) |
| `vendor/` | Written directly | Linked from the store (hardlink / reflink / copy) |
| Lock file | `composer.lock` | Same format; Composer can still read what puck writes |
| Offline install | Composer cache / `--offline` | `puck install --offline` (warm store only) |
| Self-update | `composer self-update` | `puck upgrade` (GitHub Releases manifest + minisign) |
| Laravel discovery | `artisan package:discover` | Generated natively; that Composer script is skipped when `packages.php` was written |
| Event scripts | `post-autoload-dump` / `post-install-cmd` via PHP | `puck_scripts` after install (skip with `--no-scripts`) |

## Project layout

```
crates/
  puck_cli/          CLI binary
  puck_manifest/     composer.json
  puck_lock/         composer.lock, installed.json / installed.php
  puck_install/      Install planner
  puck_autoload/     Autoload generation
  ...
fixtures/            Pinned projects for parity tests
benches/             Install timing scripts
dist/                install.sh notes, Homebrew formula generator
docs/                Install + distribution docs
```

## Development

```bash
cargo build -p puck_cli
cargo test --workspace
cargo clippy --workspace --all-targets
```

Commit messages follow `crate: description`.

## License

MIT
