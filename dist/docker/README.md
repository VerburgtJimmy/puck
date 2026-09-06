# Docker

Ship a **static musl** `puck` binary in a scratch (or distroless) image. The
repo-root [`Dockerfile`](../../Dockerfile) downloads a GitHub Release artifact,
verifies SHA-256 from `manifest.json`, and copies `/puck` into `scratch`.

## Build from a release

```bash
docker build \
  --build-arg PUCK_VERSION=v0.1.0 \
  --build-arg TARGET=x86_64-unknown-linux-musl \
  -t ghcr.io/verburgtjimmy/puck:0.1.0 .
```

Arm64:

```bash
docker build \
  --build-arg PUCK_VERSION=v0.1.0 \
  --build-arg TARGET=aarch64-unknown-linux-musl \
  -t ghcr.io/verburgtjimmy/puck:0.1.0-arm64 .
```

## COPY --from pattern (other images)

```dockerfile
COPY --from=ghcr.io/verburgtjimmy/puck:0.1.0 /puck /usr/local/bin/puck
```

Or extract once in a builder stage and copy:

```dockerfile
FROM ghcr.io/verburgtjimmy/puck:0.1.0 AS puck
FROM php:8.3-cli
COPY --from=puck /puck /usr/local/bin/puck
```

## BuildKit cache mount for the store

puck’s content-addressed store lives at `~/.puck/store` (override with
`PUCK_STORE`). In CI or repeated container builds, mount a cache so packages
are not re-downloaded:

```dockerfile
# syntax=docker/dockerfile:1.7
RUN --mount=type=cache,target=/root/.puck/store \
    puck install --working-dir /app
```

Compose / runtime:

```yaml
services:
  app:
    image: your-app
    volumes:
      - puck-store:/root/.puck/store
volumes:
  puck-store:
```

## GHCR push (optional, release workflow)

When ready, extend `.github/workflows/release.yml` with a job that builds and
pushes multi-arch images to `ghcr.io/VerburgtJimmy/puck` after artifacts are
published. Suggested sketch (not enabled in 0.1 until first tagged release):

```yaml
# docker:
#   needs: publish
#   runs-on: ubuntu-latest
#   permissions:
#     packages: write
#     contents: read
#   steps:
#     - uses: docker/setup-qemu-action@v3
#     - uses: docker/setup-buildx-action@v3
#     - uses: docker/login-action@v3
#       with:
#         registry: ghcr.io
#         username: ${{ github.actor }}
#         password: ${{ secrets.GITHUB_TOKEN }}
#     - uses: docker/build-push-action@v6
#       with:
#         push: true
#         tags: ghcr.io/verburgtjimmy/puck:${{ github.ref_name }}
#         build-args: |
#           PUCK_VERSION=${{ github.ref_name }}
#           TARGET=x86_64-unknown-linux-musl
```

Until that job exists, build and push manually from a release tag.
