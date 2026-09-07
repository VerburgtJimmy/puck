# Distribution and updates

Source of truth for how puck is built, installed, and upgraded. Versioned with
the code. Product decisions (2026-09-06) follow Fable + owner sign-off.

## Public home (0.1)

| Surface | Value |
|---|---|
| Git repository | `https://github.com/VerburgtJimmy/puck` |
| Release artifacts | GitHub Releases on that repo |
| Manifest (source of truth) | `https://github.com/VerburgtJimmy/puck/releases/latest/download/manifest.json` |
| Site / install mirror | `https://puck.jimmyverburgt.com` (`/install`, `/releases/stable.json` mirror only) |
| Org | None for 0.1 (no `quirelabs`) |

Every install / upgrade URL is a **single constant** in `install.sh` and in the
binary (`puck` crate constants). A later domain move is one constant change plus
a redirect. **Do not** switch `puck upgrade` or update checks to the subdomain
later; the GitHub Releases `manifest.json` URL is durable.

## 1. Release artifacts (per tag)

Built in GitHub Actions from the tag, never from a laptop.

| Target | Artifact | Notes |
|---|---|---|
| `aarch64-apple-darwin` | `puck-aarch64-apple-darwin.tar.gz` | **0.1:** ad-hoc codesign (`codesign -s -`). Notarization (Developer ID) is a fast follow, not a 0.1 ship gate. Supported install paths: `install.sh` and Homebrew. |
| `x86_64-apple-darwin` | `puck-x86_64-apple-darwin.tar.gz` | Same. |
| `x86_64-unknown-linux-musl` | `puck-x86_64-unknown-linux-musl.tar.gz` | Fully static. |
| `aarch64-unknown-linux-musl` | `puck-aarch64-unknown-linux-musl.tar.gz` | Fully static. |
| all | `SHA256SUMS` | One file, all artifacts. |
| all | `SHA256SUMS.minisig` | minisign over the sums file. Public key in README, site, `install.sh`, and compiled into the binary. |
| all | GitHub artifact attestation | `actions/attest-build-provenance` (`gh attestation verify`). |
| all | `manifest.json` | `{ version, released_at, channel, artifacts: { target: { url, sha256, size } } }`. What `puck upgrade` reads. |

Do **not** build glibc Linux binaries. Do **not** build Windows until Windows
support exists.

Version string embedded at build: `puck 0.1.1 (<gitsha> <date>)`.
`puck --version` prints exactly that.

**Channels:** `stable` only in 0.1. Canary is a **0.2** feature after
`parity/run.sh` has been green on every `master` push for a few weeks. Do not
define canary as `cargo test` + doctor.

## 2. `install.sh` (also mirrored at `https://puck.jimmyverburgt.com/install`)

Behaviour, in order:

1. `set -euo pipefail`. Refuse if not bash (clear message).
2. Detect OS/arch; map to a target from section 1. Unknown → list supported, exit 1. Prefer native arm64 over Rosetta.
3. Resolve version: `PUCK_VERSION` or first arg; else fetch
   `https://github.com/VerburgtJimmy/puck/releases/latest/download/manifest.json`
   (not the GitHub API). Optional mirror:
   `https://puck.jimmyverburgt.com/releases/stable.json` is never the source of truth.
4. Download tarball + `SHA256SUMS` + `SHA256SUMS.minisig`. Verify minisign if
   `minisign` is available; **always** verify sha256. On failure, delete and exit 1
   with expected vs actual.
5. Extract to `$PUCK_INSTALL/bin/puck` (`PUCK_INSTALL` default `~/.puck`). Never
   write outside that directory. Never sudo. Atomic replace (extract temp, `mv`).
6. PATH: append a commented block to the login shell rc only if needed; support
   `--no-modify-path`. Print exact line and how to remove it.
7. If Homebrew's `puck` is on PATH, refuse and print `brew upgrade puck`.
8. Print version, path, PATH note, next step: `puck doctor`.
9. `install.sh --uninstall` removes the binary and PATH block; reports store left behind.

Idempotent. Quiet success except summary. Verbose with `-v`. Manual smoke on
macOS and Linux (including uninstall and re-install) before tagging; automated
CI coverage for `install.sh` is not in place yet.

## 3. Homebrew

- Tap (0.1): the main repo — `brew tap VerburgtJimmy/puck https://github.com/VerburgtJimmy/puck`
  then `brew install puck`. Live formula is [`Formula/puck.rb`](../Formula/puck.rb),
  regenerated from `SHA256SUMS` on each release (`dist/homebrew/generate-formula.rb`
  pushes an update to `master`). `dist/homebrew/puck.rb` is a template only.
- Optional later: split to `VerburgtJimmy/homebrew-tap`.
- Core: later, after usage.
- `puck upgrade` refuses to self-update a Homebrew-managed binary.

## 4. `puck upgrade`

- Reads
  `https://github.com/VerburgtJimmy/puck/releases/latest/download/manifest.json`
  (or a pinned version URL). Channel: `stable` only in 0.1.
- Verifies sha256 always; verifies minisign when the `minisign` tool is on
  `PATH` (public key compiled in / committed under `dist/minisign/`). Atomic
  replace. Keeps `~/.puck/bin/puck.previous` for `--rollback`.
- Never automatic. Never in CI. Never touches the store or projects.

## 5. Update notifications

- At most once per 24h after a command finishes, TTY only, 500 ms timeout,
  cache in `~/.puck/cache/latest`. One line if newer.
- Disabled when `CI`, `PUCK_NO_UPDATE_CHECK=1`, `--json`, or non-TTY.
- No telemetry beyond version/target in User-Agent; say so in docs.

## 6. CI and containers

### `setup-puck` (composite action in this repo)

For 0.1 the action lives at [`.github/actions/setup-puck`](../.github/actions/setup-puck/action.yml)
so consumers can pin:

```yaml
- uses: VerburgtJimmy/puck/.github/actions/setup-puck@v0.1.1
  with:
    version: latest   # or v0.1.1
    cache: true       # caches ~/.puck/store on composer.lock hash
```

It runs the `install.sh` checked out with that action tag (no network fetch of
`master`), so script and action version match by construction. Install verifies
SHA-256 always and minisign when available, adds `~/.puck/bin` to `PATH`, and
optionally caches `~/.puck/store`.

### Docker

See [`dist/docker/README.md`](../dist/docker/README.md) and the root
[`Dockerfile`](../Dockerfile) (musl binary → `scratch`). Optional GHCR push is
documented there as a release.yml stub until the first public tag.

### GitLab / generic

`install.sh` + `PUCK_VERSION` + cache `~/.puck/store` on the lockfile hash.

## 7. Versioning

- Semver. 0.x while compatibility table has 0.2 cells.
- Changelog sections: Compatibility / Fidelity / Performance.
- Note Composer version alignment per release.

## 8. Not doing in 0.1

- No npm / pip / Packagist distribution of the binary.
- No auto-update daemon. No install-time analytics.
- No `composer` shim in the install (opt-in later).
- No canary channel.
- No *completely* unsigned macOS binary: 0.1 uses ad-hoc sign; full notarization follows quickly.

## 9. Order of work (before binaries)

1. Enable `parity/run.sh` on every push to `master`.
2. Signing keys: minisign keypair; secret in Actions; public key in README /
   `install.sh` / binary.
3. macOS ad-hoc codesign in the release workflow for 0.1. Notarization
   (Developer ID + App Store Connect API key in Actions, `spctl --assess`) is a
   fast follow after first public artifacts — not a blocker for attaching
   darwin tarballs to `v0.1.0`.
4. Release workflow: matrix build (musl Linux + darwin), sums, signature,
   attestation, `manifest.json`, GitHub release upload, tap formula push,
   optional site mirror.
5. Rewrite `install.sh` to this spec; CI test including uninstall.
6. `puck upgrade` + notification (fake manifest tests).
7. `setup-puck` action + Docker image.
8. Re-tag `v0.1.0` from the release workflow (nothing public consumes the
   current tag). Do not attach hand-built binaries.

## Constants (keep in sync)

```
MANIFEST_URL=https://github.com/VerburgtJimmy/puck/releases/latest/download/manifest.json
INSTALL_MIRROR=https://puck.jimmyverburgt.com/install
STABLE_MIRROR=https://puck.jimmyverburgt.com/releases/stable.json
REPO=VerburgtJimmy/puck
DEFAULT_INSTALL_ROOT=~/.puck
```

## Release checklist (signing)

The publish job in [`.github/workflows/release.yml`](../.github/workflows/release.yml)
reads `secrets.MINISIGN_SECRET_KEY` (full minisign secret key file contents),
writes it to a temp file, and runs `minisign -Sm SHA256SUMS -s …`. If the secret
is unset, the release still publishes with SHA-256 + attestation but without
`SHA256SUMS.minisig`.

When ready for the first public binaries: **tag `v0.1.0` from the release
workflow** (push tag `v0.1.0` to `master` after this branch is green). Do not
attach hand-built binaries. Confirm the Actions secret `MINISIGN_SECRET_KEY` is
present before tagging.
