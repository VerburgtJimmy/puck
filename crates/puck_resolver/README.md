# puck_resolver

Composer-compatible CDCL dependency solver for puck.

## Port source

This crate is a Rust port of `Composer\DependencyResolver` from
[composer/composer](https://github.com/composer/composer).

- **Upstream commit:** see [`fixtures/composer-DependencyResolver/COMPOSER_COMMIT.txt`](fixtures/composer-DependencyResolver/COMPOSER_COMMIT.txt)
- **License:** Composer is MIT (Copyright Nils Adermann, Jordi Boggiano). This
  port retains that attribution for the translated algorithms and imported
  PHPUnit fixtures under `fixtures/composer-DependencyResolver/`.
- **Decision record:** ADR 0001 in the puck-notes tree (port Composer CDCL; do
  not use PubGrub as primary).

Re-diff against upstream when Composer changes solver behaviour. Prefer
file-for-file ports of `Pool`, `PoolBuilder`, `RuleSet*`, `Decisions`, `Solver`,
`Transaction`, `Problem`, and `DefaultPolicy`.

## Determinism

Composer relies on PHP insertion-ordered arrays. This crate uses `IndexMap` /
`IndexSet` / `Vec` for structures whose iteration order can change the chosen
solution or transaction order. Do not introduce `HashMap` / `HashSet`. Sorts
that mirror PHP `sort` / `usort` use stable `slice::sort` (PHP 8+ behaviour).
