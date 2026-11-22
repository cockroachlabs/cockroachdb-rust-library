//! Vector-search integration tests. `#[ignore]`d; run with CRDB v26.2+:
//! `CRDB_URL=... cargo test -p cockroachdb-ai -- --ignored`.

use cockroachdb_ai::{rag_context, VectorConfig, VectorStore};
use cockroachdb_core::Config;
use cockroachdb_sqlx::Pool;

fn dsn() -> String {
    std::env::var("CRDB_URL")
        .unwrap_or_else(|_| "postgres://root@localhost:26257/defaultdb?sslmode=disable".into())
}

#[tokio::test]
#[ignore = "requires CockroachDB v26.2+ (set CRDB_URL)"]
async fn ensure_add_and_search_roundtrip() {
    let pool = Pool::connect(&dsn(), Config::default())
        .await
        .expect("connect");
    let store = VectorStore::new(pool.clone(), VectorConfig::new("rust_ai_docs", 3));

    store.ensure_table().await.expect("ensure table");
    // Index needs v26.2; search works brute-force without it, so this is best-effort.
    let _ = store.ensure_index().await;

    store
        .add("hello world", &[1.0, 0.0, 0.0])
        .await
        .expect("add");
    store
        .add("goodbye world", &[0.0, 1.0, 0.0])
        .await
        .expect("add");

    let hits = store.search(&[0.9, 0.1, 0.0], 1).await.expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].content, "hello world");

    let ctx = rag_context(&hits, "\n");
    assert!(ctx.contains("hello"));

    sqlx::query("DROP TABLE IF EXISTS rust_ai_docs")
        .execute(pool.raw_pool())
        .await
        .ok();
    pool.close().await;
}
