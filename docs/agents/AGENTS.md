# AGENTS.md — cockroachdb-rust-library (consumer guide)

For coding agents working in a **Rust app** that depends on this library. It
tells you what the library already does so you don't rebuild it. For full detail
see the [README](https://github.com/cockroachlabs/cockroachdb-rust-library#readme).
Every symbol below is real — don't invent others.

Entry crate consumers add: **`cockroachdb-sqlx`** (the primary adapter, over
`sqlx::PgPool`). `cockroachdb-seaorm` is the optional ORM adapter.

## 1. Already handled — do NOT rebuild it

- **Serialization retries on `SQLSTATE 40001` are automatic.** Run your unit of
  work through `Pool::execute_tx(|conn| Box::pin(async move { ... }))` (or the
  `transactional!` macro, or the `#[transactional]` attribute macro from
  `cockroachdb-macros`). Do **not** write your own retry loop or match on
  `40001` — the wrapper classifies via the error cause chain and re-runs the
  closure. Per-tx overrides: `Pool::execute_tx_with_options(ExecuteTxOptions)`.
- **Follower reads, CDC, Continuum, metrics/health are provided** — use the
  symbols in §2, don't reimplement the plumbing.

## 2. Which symbol for each job

- **Follower reads (AS OF SYSTEM TIME):** `Pool::follower_read_tx(staleness, f)`,
  `Pool::fetch_all_follower_read(query)` / `_as(staleness, query)`, the
  `follower_read!` macro, or `#[follower_read(staleness = ...)]`. `staleness ==
  Duration::ZERO` uses `follower_read_timestamp()`.
- **Region homing / regional-by-row:** homing is a **`crdb_region` COLUMN** value,
  never a `SET crdb_region` statement (no such session var). Use
  `cockroachdb_core::{RegionContext, RegionResolver, StaticRegionResolver,
  CRDB_REGION_COLUMN}`. DDL helpers: `set_primary_region`, `add_region`,
  `set_locality_regional_by_row[_as]`. On models: `#[derive(CrdbLocality)]` with
  `#[crdb(regional_by_row)]`, `#[crdb(region)]`, `#[crdb(global)]`.
- **CDC:** `cockroachdb-cdc` — changefeed SQL builder + webhook sink handler.
  Don't hand-write `CREATE CHANGEFEED` plumbing.
- **AI / vector search:** `cockroachdb-ai` — pgvector `VECTOR` / C-SPANN search.
- **Continuum (vCluster, Plenum, Aegis, MCP, Cloud):** `cockroachdb-continuum`.

## 3. Config that matters

`cockroachdb_core::Config` / the SQLx `Pool::connect(conn_str, config)` builder:
connection string, retry max attempts, follower-read `staleness`, region. Force
`SERIALIZABLE` isolation if you depend on 40001 firing (READ COMMITTED hides it).

## 4. Anti-patterns

- Hand-rolled 40001 retry loops — `execute_tx` already does it.
- Emitting a raw `SET crdb_region` — set the `crdb_region` **column** instead.
- Writing changefeed plumbing by hand — use `cockroachdb-cdc`.
- Running under READ COMMITTED while expecting retries — 40001 won't surface.

## 5. Testing

Use `cockroachdb-testing` — test harness, concurrency runner, retry/follower
simulators (no live cluster required).
