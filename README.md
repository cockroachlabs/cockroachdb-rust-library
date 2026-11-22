# cockroachdb-rust-library

Rust client library for [CockroachDB](https://www.cockroachlabs.com/): automatic
transaction retries on serialization conflicts (`SQLSTATE 40001`), follower reads,
multi-region row homing, CDC changefeeds, AI/vector search, Continuum support,
metrics, circuit breaker, and health checks.

The Rust sibling of the
[Spring Boot Starter](https://github.com/cockroachlabs/cockroachdb-spring-boot-starter),
[Go](https://github.com/cockroachlabs/cockroachdb-go-library),
[Python](https://github.com/cockroachlabs/cockroachdb-python-library), and
[.NET](https://github.com/cockroachlabs/cockroachdb-dotnet-library) libraries.
It follows the same **core + adapter** shape: a driver-agnostic core, a primary
[SQLx](https://github.com/launchbadge/sqlx) adapter, and an optional
[SeaORM](https://www.sea-ql.org/SeaORM/) adapter.

> Rust has no runtime AOP, so the Spring annotations map to **closures**
> (`pool.execute_tx(|conn| ...)`, the Go model) with optional attribute macros
> (`#[cockroachdb::transactional]`) as annotation-style sugar.

## Workspace crates

| Crate | Purpose |
|-------|---------|
| `cockroachdb-core` | Driver-agnostic: retry policy, SQLSTATE classification, isolation, follower-read AOST builders, region (`crdb_region` column homing), middleware, circuit breaker, metrics/health traits, DDL helpers. |
| `cockroachdb-sqlx` | **Primary adapter.** `Pool` over `sqlx::PgPool`, `execute_tx` retry loop, follower-read queries, SQLSTATE classification for sqlx errors. |
| `cockroachdb-seaorm` | ORM adapter (ActiveModel) built on the SQLx core. |
| `cockroachdb-cdc` | Changefeed SQL builder + webhook sink handler + metrics. |
| `cockroachdb-ai` | pgvector `VECTOR` / C-SPANN similarity search + semantic/RAG. |
| `cockroachdb-continuum` | Continuum support: vCluster, Plenum, Aegis, MCP, Cloud. |
| `cockroachdb-macros` | Attribute macros (`#[transactional]`, `#[follower_read]`) — the AOP analogue. |
| `cockroachdb-testing` | Test harness, concurrency runner, retry/follower simulators (no live cluster). |

## Installation

Served from this repository's GitHub Pages **sparse Cargo registry** — no token
required. In `.cargo/config.toml`:

```toml
[registries.cockroachlabs]
index = "sparse+https://cockroachlabs.github.io/cockroachdb-rust-library/registry/"
```

```toml
# Cargo.toml
[dependencies]
cockroachdb-sqlx = { version = "0.1", registry = "cockroachlabs" }
```

## License

Apache-2.0. See [LICENSE](LICENSE).
