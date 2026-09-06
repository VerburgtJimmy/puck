# puck

Native PHP package installer. Reads the same `composer.json` / `composer.lock` as Composer, writes a compatible `vendor/` tree, and aims for byte-identical output (autoload files, `installed.json`, `installed.php`) without running PHP on the install path.

puck is not a full Composer replacement yet. Keep Composer installed; puck reads and writes the same files and you can switch back at any time.

## Status

**v0.1.0** — `puck install`, `require`, `remove`, `update` / `lock` on macOS and Linux, with Tier 1 native adapters for Pest and phpstan/extension-installer. Run `puck doctor` before switching a project.

## Benchmarks (Linux CI)

Lead with **warm-wipe** (vendor wiped, cache/store warm). Warm-keep is a no-op demo. Do not lead with cold.

### laravel-skeleton `--no-dev`

| scenario | composer (ms) | puck (ms) |
|---|---:|---:|
| cold (empty vendor) | 4864 | 5805 |
| warm (vendor present) | 756 | 20 |
| warm (wipe vendor, cache/store warm) | 1618 | 226 |

### laravel-app with require-dev

| scenario | composer (ms) | puck (ms) |
|---|---:|---:|
| cold (empty vendor) | 4692 | 1854 |
| warm (vendor present) | 1534 | 91 |
| warm (wipe vendor, cache/store warm) | 3623 | 528 |

Warm-wipe on laravel-app with-dev is dominated by the optimized classmap dump, which scales with require-dev.

Warm-keep 91 ms (Linux CI above) predates the O(1) keep path (skip vendor walk + bin rewrite). Local remeasure after the fix: ~4 ms on macOS with-dev; expect Linux warm-keep closer to skeleton (~20–40 ms) once re-benched.

Methodology and more runs: [`benches/`](benches/) (`install-timing.sh`, `resource-usage.sh`, CI workflow).

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
curl -fsSL https://raw.githubusercontent.com/VerburgtJimmy/puck/master/install.sh | bash
```

Installs a release binary into `~/.local/bin` (or `/usr/local/bin` if writable). See [`docs/install.md`](docs/install.md).

### Homebrew

```bash
brew tap VerburgtJimmy/puck https://github.com/VerburgtJimmy/puck
brew install puck
```

Formula: [`dist/homebrew/puck.rb`](dist/homebrew/puck.rb). Release assets must be attached to the GitHub release (see [`dist/README.md`](dist/README.md)).

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
puck store path              # print the global store path
puck store gc                # garbage-collect unused store entries
```

Set `PUCK_TIMINGS=1` to print phase timings on stderr (`plan_ms`, `link_ms`, `dump_ms`, …).

## What 0.1 does not do yet

- VCS / artifact / package repositories
- Windows
- Filter-list / malware locked-package checks
- Broader plugin coverage (Tier 2 PHP plugin host; more Tier 1 adapters)
- `self-update` polish

## How it differs

| | Composer | puck |
|---|---|---|
| Runtime for install | PHP | Native (Rust) |
| Package cache | Per-project / Composer cache | Shared content-addressable store (`~/.puck/store`) |
| `vendor/` | Written directly | Linked from the store (hardlink / reflink / copy) |
| Lock file | `composer.lock` | Same format; Composer can still read what puck writes |
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
dist/                install.sh notes, Homebrew formula
docs/                Install docs
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
