//! Virtual-cluster quality-of-service tagging.
//!
//! Applies a vCluster QoS and application name to a session/transaction so a
//! shared CockroachDB cluster can prioritise or de-prioritise the work.

use cockroachdb_sqlx::Pool;

/// Transaction quality-of-service level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Priority {
    /// Default scheduling.
    #[default]
    Regular,
    /// Elevated priority for latency-sensitive work.
    Critical,
    /// Deprioritised, yields to higher-priority work.
    Background,
}

impl Priority {
    /// The `default_transaction_quality_of_service` value for this priority.
    pub fn qos(&self) -> &'static str {
        match self {
            Priority::Regular => "regular",
            Priority::Critical => "critical",
            Priority::Background => "background",
        }
    }
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.qos())
    }
}

/// A vCluster tagging context: an application name and a priority.
#[derive(Debug, Clone, Default)]
pub struct VClusterContext {
    /// Application name reported to the cluster (for attribution).
    pub application: Option<String>,
    /// QoS priority for transactions run under this context.
    pub priority: Priority,
}

impl VClusterContext {
    /// A context for `application` at the default (regular) priority.
    pub fn new(application: impl Into<String>) -> Self {
        Self {
            application: Some(application.into()),
            priority: Priority::Regular,
        }
    }

    /// Sets the priority.
    pub fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// The `SET` statements that apply this context. Run them as the first
    /// statements inside a transaction (see [`VClusterContext::apply_to`]).
    pub fn apply_statements(&self) -> Vec<String> {
        let mut stmts = vec![format!(
            "SET default_transaction_quality_of_service = '{}'",
            self.priority.qos()
        )];
        if let Some(app) = &self.application {
            stmts.push(format!(
                "SET application_name = '{}'",
                app.replace('\'', "''")
            ));
        }
        stmts
    }

    /// Applies the context to a connection (e.g. the one inside an `execute_tx`
    /// closure: `ctx.apply_to(conn).await?;` before your queries).
    pub async fn apply_to(&self, conn: &mut sqlx::PgConnection) -> Result<(), sqlx::Error> {
        for stmt in self.apply_statements() {
            sqlx::query(&stmt).execute(&mut *conn).await?;
        }
        Ok(())
    }

    /// Applies the context at session scope on a pooled connection.
    pub async fn apply_session(&self, pool: &Pool) -> Result<(), sqlx::Error> {
        for stmt in self.apply_statements() {
            sqlx::query(&stmt).execute(pool.raw_pool()).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qos_strings() {
        assert_eq!(Priority::Regular.qos(), "regular");
        assert_eq!(Priority::Critical.qos(), "critical");
        assert_eq!(Priority::Background.qos(), "background");
    }

    #[test]
    fn apply_statements_include_qos_and_app() {
        let ctx = VClusterContext::new("payroll").with_priority(Priority::Background);
        let stmts = ctx.apply_statements();
        assert_eq!(
            stmts[0],
            "SET default_transaction_quality_of_service = 'background'"
        );
        assert_eq!(stmts[1], "SET application_name = 'payroll'");
    }

    #[test]
    fn app_name_is_escaped() {
        let ctx = VClusterContext::new("pay'roll");
        let stmts = ctx.apply_statements();
        assert_eq!(stmts[1], "SET application_name = 'pay''roll'");
    }

    #[test]
    fn no_app_only_sets_qos() {
        let ctx = VClusterContext::default();
        assert_eq!(ctx.apply_statements().len(), 1);
    }
}
