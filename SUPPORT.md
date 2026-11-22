# Support

The CockroachDB Rust library is maintained by Cockroach Labs and distributed under
Apache-2.0.

## Getting help

- **Bugs / feature requests** — open a GitHub issue using the templates in
  `.github/ISSUE_TEMPLATE/`. A minimal reproduction (library version, driver,
  CockroachDB version, cluster topology, and a short code sample) gets you a
  faster answer.
- **Questions / usage** — start a GitHub Discussion or ask in the CockroachDB
  community Slack.
- **Security issues** — do **not** open a public issue. Email
  security@cockroachlabs.com.

## In scope

- The crates in this workspace: `cockroachdb-core`, `cockroachdb-sqlx`,
  `cockroachdb-seaorm`, `cockroachdb-cdc`, `cockroachdb-ai`,
  `cockroachdb-continuum`, `cockroachdb-macros`, `cockroachdb-testing`.
- Behaviour specific to CockroachDB: serialization-conflict retries (`40001`),
  follower reads (`AS OF SYSTEM TIME`), multi-region row homing, CDC, vector
  search, vCluster QoS, and the metrics/health surface.

## Out of scope

- Bugs in SQLx, SeaORM, or the Postgres wire protocol themselves — report those
  upstream (we'll help identify where).
- General Rust/async questions unrelated to this library.

## Compatibility

| Area | Supported |
|------|-----------|
| Rust | 1.75+ (edition 2021) |
| CockroachDB | v23.2+ for retries/follower reads/multi-region; **v26.2+** for `CREATE VECTOR INDEX` (C-SPANN) |
| SQLx | 0.8.x |
| SeaORM | 1.x |

See the docs site for the full version-compatibility matrix.
