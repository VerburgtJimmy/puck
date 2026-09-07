# minisign keys for puck releases

Release artifacts are signed with [minisign](https://jedisct1.github.io/minisign/).
`install.sh` and `puck upgrade` **always** verify SHA-256 of the artifact against
`SHA256SUMS`. They verify `SHA256SUMS.minisig` when the `minisign` tool is on
`PATH`; otherwise they warn and continue with checksum-only trust.

## Public key (committed)

File: [`minisign.pub`](minisign.pub)

```
untrusted comment: minisign public key D78B3C3D36ED964D
RWRNlu02PTyL13V6QL9fxE4Ho6fcGHI/5fu6HdGzQ1mlKghwOWDg/6ft
```

Inline (same key):

```
RWRNlu02PTyL13V6QL9fxE4Ho6fcGHI/5fu6HdGzQ1mlKghwOWDg/6ft
```

## Generate a new keypair (maintainers)

```bash
brew install minisign   # or https://jedisct1.github.io/minisign/
minisign -G -W -p dist/minisign/minisign.pub -s .secrets/minisign.key
```

- Commit **only** `minisign.pub` (and this README).
- Never commit the secret key. Keep it in a local gitignored path (e.g. `.secrets/minisign.key`).

## GitHub Actions secret

Upload the **entire** secret key file contents as repository secret:

| Secret name | Value |
|---|---|
| `MINISIGN_SECRET_KEY` | Full contents of the minisign secret key file |

The release workflow signs `SHA256SUMS` when this secret is present. Releases still publish without it (unsigned sums), but signed releases are required before treating install as production-ready.
