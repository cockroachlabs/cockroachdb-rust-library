# Contributing

Thanks for helping improve the CockroachDB Rust library.

## Workspace layout

A Cargo workspace of eight crates plus a reference app:

```
crates/
  cockroachdb-core        # driver-agnostic: retry, follower, region, circuit breaker, metrics/health traits, ddl
  cockroachdb-sqlx        # primary adapter: Pool, execute_tx, follower reads
  cockroachdb-seaorm      # ORM adapter on the SQLx core
  cockroachdb-cdc         # changefeed builder + webhook sink
  cockroachdb-ai          # pgvector search + RAG retrieval
  cockroachdb-continuum   # vCluster / Plenum / Aegis / MCP / Cloud
  cockroachdb-macros      # #[transactional] / #[follower_read] attribute macros
  cockroachdb-testing     # concurrency runner, retry simulator, fake errors
examples/ideas-board      # quickstart reference app
```

## Build & test

```bash
cargo build                       # whole workspace
cargo test                        # unit + doc tests (no database needed)
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

Database-dependent tests are marked `#[ignore]`. Run them against a cluster:

```bash
docker run -d --name crdb -p 26257:26257 cockroachdb/cockroach:latest \
  start-single-node --insecure
export CRDB_URL="postgres://root@localhost:26257/defaultdb?sslmode=disable"
cargo test -- --ignored
```

The vector-search integration test needs **CockroachDB v26.2+** for
`CREATE VECTOR INDEX`.

## Conventions

- Match the Go library's API names and semantics where a parallel exists — the
  docs describe both consistently.
- Every non-trivial change ships a test. Keep the driver-agnostic logic in
  `cockroachdb-core` and driver specifics in the adapters.
- No AI attribution in commits or PRs.

## Releasing

Versions come from the workspace manifest (`[workspace.package] version`), bumped
via PR to `main` (the branch is protected). On push to `main`, `pages.yml` builds
the sparse Cargo registry and rustdoc and deploys them to GitHub Pages — that feed
is what customers install from. See [PACKAGING.md](PACKAGING.md).
