# syntax=docker/dockerfile:1.7
#
# Static musl puck binary on scratch.
#
#   docker build --build-arg PUCK_VERSION=v0.1.0 \
#     --build-arg TARGET=x86_64-unknown-linux-musl \
#     -t ghcr.io/verburgtjimmy/puck:0.1.0 .

ARG TARGET=x86_64-unknown-linux-musl
ARG PUCK_VERSION=latest

FROM alpine:3.20 AS downloader
ARG TARGET
ARG PUCK_VERSION
RUN apk add --no-cache ca-certificates python3
WORKDIR /out
RUN set -euo pipefail; \
  if [ "${PUCK_VERSION}" = "latest" ]; then \
    MANIFEST_URL="https://github.com/VerburgtJimmy/puck/releases/latest/download/manifest.json"; \
  else \
    case "${PUCK_VERSION}" in v*) TAG="${PUCK_VERSION}" ;; *) TAG="v${PUCK_VERSION}" ;; esac; \
    MANIFEST_URL="https://github.com/VerburgtJimmy/puck/releases/download/${TAG}/manifest.json"; \
  fi; \
  python3 - "$TARGET" "$MANIFEST_URL" <<'PY'
import hashlib, io, json, pathlib, sys, tarfile, urllib.request
target, manifest_url = sys.argv[1], sys.argv[2]
manifest = json.load(urllib.request.urlopen(manifest_url))
art = manifest["artifacts"][target]
data = urllib.request.urlopen(art["url"]).read()
actual = hashlib.sha256(data).hexdigest()
if actual != art["sha256"]:
    raise SystemExit(f"sha256 mismatch: expected {art['sha256']}, got {actual}")
tf = tarfile.open(fileobj=io.BytesIO(data), mode="r:gz")
member = next(m for m in tf.getmembers() if pathlib.Path(m.name).name == "puck")
out = pathlib.Path("/out/puck")
out.write_bytes(tf.extractfile(member).read())
out.chmod(0o755)
print(f"installed puck {manifest['version']} ({target})")
PY

FROM scratch
COPY --from=downloader /out/puck /puck
ENTRYPOINT ["/puck"]
