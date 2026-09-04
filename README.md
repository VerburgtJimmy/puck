# puck

A fast, native package manager for PHP. Laravel-first, Composer-compatible.

`puck` is a single static binary that installs from your existing `composer.json` and `composer.lock`. It keeps a content-addressable shared store, links packages into a normal `vendor/` directory, and aims for byte-identical output with Composer - including autoload files, `installed.json`, and `installed.php` - without running PHP on the install path.

## Status

Early development. The public milestone is `puck install` from a lock file (parity with Composer on Laravel fixtures, with benchmarks). Commands below that are not yet implemented will exit with a clear error.

## Install

Installation methods (script, Homebrew, packages) will ship closer to launch. For now, build from source:

```bash
git clone https://github.com/jimmyverburgt/puck.git
cd puck
cargo build --release -p puck_cli
```

The binary is `target/release/puck`.

Requires Rust 1.85+.

## Usage

```bash
puck install                 # install from composer.lock
puck install --no-dev        # production install
puck install --offline       # warm store only; no network
puck dump-autoload -o        # regenerate optimized autoload
puck remove vendor/package
puck store path              # print the global store path
puck store gc                # garbage-collect unused store entries
```

`update` and `require` (dependency resolution) come later. Until then, generate or refresh lock files with Composer as needed; `puck install` consumes them.

## How it differs

| | Composer | puck |
|---|---|---|
| Runtime for install | PHP | Native (Rust) |
| Package cache | Per-project / Composer cache | Shared content-addressable store (`~/.puck/store`) |
| `vendor/` | Written directly | Linked from the store (hardlink / reflink / copy) |
| Lock file | `composer.lock` | Same format; Composer can still read what puck writes |
| Laravel discovery | `artisan package:discover` | Generated natively when the standard script is detected |

Composer remains the reference for dependency semantics. Deliberate differences will be documented as they land.

## Project layout

```
crates/
  puck_cli/          CLI binary
  puck_manifest/     composer.json
  puck_lock/         composer.lock, installed.json / installed.php
  puck_version/      Composer version & constraint rules
  puck_registry/     Packagist and private repos
  puck_dist/         Archive download & extract
  puck_store/        Global store & linking
  puck_install/      Install planner
  puck_autoload/     Autoload generation
  puck_laravel/      Package discovery & Laravel helpers
  ...
fixtures/            Pinned projects for parity tests
parity/              Composer vs puck comparison harness
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
