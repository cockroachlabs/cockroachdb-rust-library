//! Pure SQL builders for vector search and schema setup.
//!
//! These produce strings only — [`crate::VectorStore`] executes them — so they
//! are fully unit-testable without a database.

use crate::distance::DistanceMetric;

/// Quotes a SQL identifier.
pub fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Formats a vector as a CockroachDB vector literal body, e.g. `[0.1,0.2,0.3]`.
/// Bind this as a `$n` parameter and cast with `::vector` in the query.
pub fn vector_literal(v: &[f32]) -> String {
    let mut s = String::with_capacity(v.len() * 8 + 2);
    s.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&x.to_string());
    }
    s.push(']');
    s
}

/// `CREATE TABLE IF NOT EXISTS` for a vector store with `id`, `content`,
/// `embedding VECTOR(dims)`, and a `metadata JSONB` column.
pub fn create_table(
    table: &str,
    id_col: &str,
    content_col: &str,
    embedding_col: &str,
    dims: u32,
) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {t} (\
         {id} STRING PRIMARY KEY DEFAULT gen_random_uuid()::STRING, \
         {content} STRING NOT NULL, \
         {emb} VECTOR({dims}), \
         metadata JSONB)",
        t = ident(table),
        id = ident(id_col),
        content = ident(content_col),
        emb = ident(embedding_col),
        dims = dims,
    )
}

/// `CREATE VECTOR INDEX IF NOT EXISTS` (C-SPANN). Requires CockroachDB v26.2+.
pub fn create_vector_index(index_name: &str, table: &str, embedding_col: &str) -> String {
    format!(
        "CREATE VECTOR INDEX IF NOT EXISTS {idx} ON {t} ({emb})",
        idx = ident(index_name),
        t = ident(table),
        emb = ident(embedding_col),
    )
}

/// `INSERT ... RETURNING id`. The vector is bound as `$2::vector`, metadata as
/// `$3::jsonb` (bind a JSON string, or `NULL`).
pub fn insert(table: &str, id_col: &str, content_col: &str, embedding_col: &str) -> String {
    format!(
        "INSERT INTO {t} ({content}, {emb}, metadata) VALUES ($1, $2::vector, $3::jsonb) RETURNING {id}::text",
        t = ident(table),
        content = ident(content_col),
        emb = ident(embedding_col),
        id = ident(id_col),
    )
}

/// A `SELECT ... ORDER BY embedding <op> $1::vector LIMIT $2` search.
///
/// `$1` is the query vector literal, `$2` the limit. An optional `filter_sql` is
/// spliced into a `WHERE` clause (its own binds continue from `$3`).
pub fn search(
    table: &str,
    id_col: &str,
    content_col: &str,
    embedding_col: &str,
    metric: DistanceMetric,
    filter_sql: Option<&str>,
) -> String {
    let op = metric.operator();
    let emb = ident(embedding_col);
    let where_clause = match filter_sql {
        Some(f) if !f.trim().is_empty() => format!(" WHERE {}", f),
        _ => String::new(),
    };
    format!(
        "SELECT {id}::text AS id, {content} AS content, {emb} {op} $1::vector AS distance \
         FROM {t}{where_clause} ORDER BY {emb} {op} $1::vector LIMIT $2",
        id = ident(id_col),
        content = ident(content_col),
        emb = emb,
        op = op,
        t = ident(table),
        where_clause = where_clause,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_literal_format() {
        assert_eq!(vector_literal(&[0.1, 0.2, 0.3]), "[0.1,0.2,0.3]");
        assert_eq!(vector_literal(&[]), "[]");
    }

    #[test]
    fn create_table_sql() {
        let sql = create_table("docs", "id", "content", "embedding", 1536);
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS \"docs\""));
        assert!(sql.contains("\"embedding\" VECTOR(1536)"));
        assert!(sql.contains("metadata JSONB"));
    }

    #[test]
    fn vector_index_sql() {
        assert_eq!(
            create_vector_index("docs_emb_idx", "docs", "embedding"),
            "CREATE VECTOR INDEX IF NOT EXISTS \"docs_emb_idx\" ON \"docs\" (\"embedding\")"
        );
    }

    #[test]
    fn search_sql_l2() {
        let sql = search(
            "docs",
            "id",
            "content",
            "embedding",
            DistanceMetric::L2,
            None,
        );
        assert_eq!(
            sql,
            "SELECT \"id\"::text AS id, \"content\" AS content, \"embedding\" <-> $1::vector AS distance \
             FROM \"docs\" ORDER BY \"embedding\" <-> $1::vector LIMIT $2"
        );
    }

    #[test]
    fn search_sql_with_filter_and_metric() {
        let sql = search(
            "docs",
            "id",
            "content",
            "embedding",
            DistanceMetric::Cosine,
            Some("metadata->>'lang' = $3"),
        );
        assert!(sql.contains("<=>"));
        assert!(sql.contains("WHERE metadata->>'lang' = $3"));
    }
}
