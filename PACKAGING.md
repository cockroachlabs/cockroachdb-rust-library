# Packaging & distribution

The CockroachDB Rust library ships the same way as the other developer
libraries: an **anonymous, no-token feed served from GitHub Pages**. For Rust
that feed is a **sparse Cargo registry** — the direct analogue of the Spring/Java
Maven feed, the Python PEP 503 index, and the .NET static NuGet feed.

> We deliberately do **not** publish to crates.io (the public-registry equivalent
> of PyPI.org / NuGet.org, which this family avoids). A git dependency is the
> zero-infrastructure fallback (see the end of this doc).

## How consumers install

In `.cargo/config.toml` (project or `~/.cargo/config.toml`):

```toml
[registries.cockroachlabs]
index = "sparse+https://cockroachlabs.github.io/cockroachdb-rust-library/registry/"
```

Then in `Cargo.toml`:

```toml
[dependencies]
cockroachdb-sqlx = { version = "0.1", registry = "cockroachlabs" }
```

No token, no auth — same as `cargo add`. `cargo` fetches the index over plain
HTTPS and the `.crate` tarballs from the same Pages site.

## What the feed contains

```
registry/config.json              # { "dl": ".../crates/{crate}/{crate}-{version}.crate", "api": null }
registry/<prefix>/<name>          # JSON-lines index, one line per version (e.g. co/ck/cockroachdb-sqlx)
crates/<name>/<name>-<ver>.crate  # the packaged tarballs
doc/                              # rustdoc for every crate
index.html                        # landing page
```

A sparse registry is **just static files** — exactly what GitHub Pages serves.

## How it is built — `scripts/build-registry.py`

Run by `.github/workflows/pages.yml` on push to `main` (and on release /
manual dispatch). The script:

1. reads `cargo metadata` and selects the publishable workspace crates
   (`examples/ideas-board` is `publish = false`, so it is excluded);
2. topologically sorts them so every internal dependency is packaged first;
3. serves the registry-in-progress over `127.0.0.1` and points
   `CARGO_REGISTRIES_COCKROACHLABS_INDEX` at it, so `cargo package` can resolve
   each internal dependency from the entries already written — this solves the
   self-hosted-registry bootstrap (a fresh feed with no prior versions);
4. runs `cargo package --no-verify` per crate, copies the `.crate`, and writes an
   index line with the sha256 checksum and the dependency set.

**The dependency classification is the subtle part** and is verified in the
generated index: an internal dependency gets `registry: null` (meaning "this same
registry"), while every crates.io dependency gets
`registry: https://github.com/rust-lang/crates.io-index`. Getting that wrong would
make `cargo` look for `tokio`/`sqlx` in our feed.

Internal dependencies are declared in the root `Cargo.toml` as
`{ path = "...", version = "0.1.0", registry = "cockroachlabs" }`: the `path` wins
for local builds, while `version` + `registry` are what `cargo package` records
for consumers. The repo's own `.cargo/config.toml` defines the `cockroachlabs`
registry so those manifests parse locally.

Build the feed locally to inspect it (no publish):

```bash
python3 scripts/build-registry.py --out _site
```

## Versioning & release

- Versions come from `[workspace.package] version` in the root `Cargo.toml` (all
  crates share one version), bumped via PR to the protected `main` branch.
- Merging the bump to `main` triggers `pages.yml`, which rebuilds the feed and
  rustdoc — that deploy **is** the release customers install from. The feed is
  rebuilt fresh each deploy (single-version, like the Maven feed).

## Zero-infrastructure fallback: git dependency

Exactly like the Go library, consumers can skip the registry entirely:

```toml
[dependencies]
cockroachdb-sqlx = { git = "https://github.com/cockroachlabs/cockroachdb-rust-library", tag = "v0.1.0" }
```

This needs no `pages.yml` registry step, but loses `cargo add --registry` and
versioned resolution. The sparse registry is the primary, family-consistent path.
