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

## Learn CockroachDB fundamentals

The library handles the Rust side; these CockroachDB docs explain the database
underneath. Each link points at the Markdown version of the page (easier for an
agent to read); drop the `.md` for the HTML page. Every docs page has one, and
<https://docs.cockroachlabs.com/llms.txt> indexes the whole documentation site.

**Start here**

- [Developer Basics](https://docs.cockroachlabs.com/docs/stable/developer-basics.md): what's different about building on CockroachDB.
- [PostgreSQL Compatibility](https://docs.cockroachlabs.com/docs/stable/postgresql-compatibility.md): CockroachDB speaks the PostgreSQL wire protocol (SQLx's `postgres` driver connects to it), with some differences.
- [Connection Pooling](https://docs.cockroachlabs.com/docs/stable/connection-pooling.md): sizing a `sqlx::PgPool`.
- [Common Errors and Solutions](https://docs.cockroachlabs.com/docs/stable/common-errors.md)

**Transactions and contention**

- [Transactions](https://docs.cockroachlabs.com/docs/stable/transactions.md): SERIALIZABLE by default, and why clients see retry errors.
- [Transaction Retry Error Reference](https://docs.cockroachlabs.com/docs/stable/transaction-retry-error-reference.md): what each `40001` (`RETRY_SERIALIZABLE`, `RETRY_WRITE_TOO_OLD`, ...) means. `execute_tx` retries them; this tells you why they happen.
- [Reduce transaction contention](https://docs.cockroachlabs.com/docs/stable/performance-best-practices-overview.md#reduce-transaction-contention): keep transactions small, limit the rows they touch, use `SELECT ... FOR UPDATE` for read-then-write, `UPSERT` whole rows, use historical reads where staleness is fine.
- [Understand Hotspots](https://docs.cockroachlabs.com/docs/stable/understand-hotspots.md)
- [Performance Tuning Recipes](https://docs.cockroachlabs.com/docs/stable/performance-recipes.md)
- [`SELECT FOR UPDATE`](https://docs.cockroachlabs.com/docs/stable/select-for-update.md)
- [Read Committed Transactions](https://docs.cockroachlabs.com/docs/stable/read-committed.md): an alternative isolation level, with trade-offs. Don't switch to it just to avoid retries — it hides `40001`.

**Schema design**

- [Database Schemas](https://docs.cockroachlabs.com/docs/stable/schema-design-overview.md): design and change schemas with a migration tool rather than through the ORM.
- [Unique ID best practices](https://docs.cockroachlabs.com/docs/stable/performance-best-practices-overview.md#unique-id-best-practices): prefer `UUID` keys (`gen_random_uuid()`) over sequences or auto-increment, which create hotspots.
- [`UUID`](https://docs.cockroachlabs.com/docs/stable/uuid.md) and [Hash-sharded Indexes](https://docs.cockroachlabs.com/docs/stable/hash-sharded-indexes.md), for when an index must be on a sequential column.
- [Online Schema Changes](https://docs.cockroachlabs.com/docs/stable/online-schema-changes.md)

**Multi-region**

- [Multi-Region Capabilities Overview](https://docs.cockroachlabs.com/docs/stable/multiregion-overview.md)
- [Table Localities](https://docs.cockroachlabs.com/docs/stable/table-localities.md): `REGIONAL BY TABLE`, `REGIONAL BY ROW` and `GLOBAL`, which `#[crdb(regional_by_row)]` and `#[crdb(global)]` map to.
- [How to Choose a Multi-Region Configuration](https://docs.cockroachlabs.com/docs/stable/choosing-a-multi-region-configuration.md)
- [Follower Reads](https://docs.cockroachlabs.com/docs/stable/follower-reads.md) and [`AS OF SYSTEM TIME`](https://docs.cockroachlabs.com/docs/stable/as-of-system-time.md), behind `follower_read_tx`.

**Change data capture**

- [`CREATE CHANGEFEED`](https://docs.cockroachlabs.com/docs/stable/create-changefeed.md) and [Changefeed Sinks](https://docs.cockroachlabs.com/docs/stable/changefeed-sinks.md) (the webhook sink is what `cockroachdb-cdc` receives).

**Vector search**

- [`VECTOR`](https://docs.cockroachlabs.com/docs/stable/vector.md) and [Vector Indexes](https://docs.cockroachlabs.com/docs/stable/vector-indexes.md)
