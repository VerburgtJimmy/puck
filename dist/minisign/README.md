# minisign keys for puck releases

Release artifacts are signed with [minisign](https://jedisct1.github.io/minisign/).
`install.sh` and `puck upgrade` verify `SHA256SUMS.minisig` when `minisign` is available.

## Public key (committed)

File: [`minisign.pub`](minisign.pub)

```
untrusted comment: minisign public key 6635B0F2C6E694F9
RWT5lObG8rA1ZuBXvOCGdlfqQ4FdAK0l9VdFTH2J3zPVaC8jlo+DvmHa
```

Inline (same key):

```
RWT5lObG8rA1ZuBXvOCGdlfqQ4FdAK0l9VdFTH2J3zPVaC8jlo+DvmHa
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
