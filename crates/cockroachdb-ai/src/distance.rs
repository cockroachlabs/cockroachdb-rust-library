//! Vector distance metrics and their CockroachDB operators.

/// A vector similarity metric. The operator is the pgvector-compatible distance
/// operator CockroachDB understands; smaller values rank first in all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DistanceMetric {
    /// Euclidean (L2) distance — operator `<->`.
    #[default]
    L2,
    /// Cosine distance — operator `<=>`.
    Cosine,
    /// Negative inner product — operator `<#>`.
    InnerProduct,
}

impl DistanceMetric {
    /// The distance operator for use in `ORDER BY embedding <op> $1`.
    pub fn operator(&self) -> &'static str {
        match self {
            DistanceMetric::L2 => "<->",
            DistanceMetric::Cosine => "<=>",
            DistanceMetric::InnerProduct => "<#>",
        }
    }
}

impl std::fmt::Display for DistanceMetric {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.operator())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operators() {
        assert_eq!(DistanceMetric::L2.operator(), "<->");
        assert_eq!(DistanceMetric::Cosine.operator(), "<=>");
        assert_eq!(DistanceMetric::InnerProduct.operator(), "<#>");
        assert_eq!(DistanceMetric::default(), DistanceMetric::L2);
    }
}
