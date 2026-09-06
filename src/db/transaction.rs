use super::{DbType, QueryResult, QueryRowSink, StatementResult};
use async_trait::async_trait;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TransactionIsolation {
    #[default]
    DatabaseDefault,
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

impl TransactionIsolation {
    pub fn sql(self) -> Option<&'static str> {
        match self {
            Self::DatabaseDefault => None,
            Self::ReadCommitted => Some("READ COMMITTED"),
            Self::RepeatableRead => Some("REPEATABLE READ"),
            Self::Serializable => Some("SERIALIZABLE"),
        }
    }
}

impl DbType {
    pub fn transaction_isolations(self) -> &'static [TransactionIsolation] {
        use TransactionIsolation::*;
        match self {
            Self::PostgreSQL | Self::MySQL | Self::SQLServer => {
                &[DatabaseDefault, ReadCommitted, RepeatableRead, Serializable]
            }
            Self::SQLite => &[DatabaseDefault, Serializable],
            Self::ClickHouse | Self::MongoDB | Self::Redis => &[],
        }
    }
}

#[async_trait]
pub trait DatabaseTransaction: Send + Sync {
    fn db_type(&self) -> DbType;
    async fn execute(&mut self, sql: &str) -> anyhow::Result<QueryResult>;
    async fn execute_stream(
        &mut self,
        sql: &str,
        sink: &mut dyn QueryRowSink,
    ) -> anyhow::Result<QueryResult>;
    async fn commit(self: Box<Self>) -> anyhow::Result<()>;
    async fn rollback(self: Box<Self>) -> anyhow::Result<()>;

    async fn apply_batch(
        &mut self,
        statements: Vec<String>,
    ) -> anyhow::Result<Vec<StatementResult>> {
        let sql_server = self.db_type() == DbType::SQLServer;
        self.execute(if sql_server {
            "SAVE TRANSACTION astesia_grid_batch"
        } else {
            "SAVEPOINT astesia_grid_batch"
        })
        .await?;
        let mut results = Vec::with_capacity(statements.len());
        for sql in statements {
            let started = std::time::Instant::now();
            match self.execute(&sql).await {
                Ok(result) => results.push(StatementResult::from_query_result(sql, result)),
                Err(error) => {
                    self.execute(if sql_server {
                        "ROLLBACK TRANSACTION astesia_grid_batch"
                    } else {
                        "ROLLBACK TO SAVEPOINT astesia_grid_batch"
                    })
                    .await
                    .map_err(|rollback| {
                        anyhow::anyhow!(
                            "Batch failed: {error}; savepoint recovery failed: {rollback}"
                        )
                    })?;
                    if !sql_server {
                        self.execute("RELEASE SAVEPOINT astesia_grid_batch").await?;
                    }
                    results.push(StatementResult::from_error(
                        sql,
                        error,
                        started.elapsed().as_millis() as u64,
                    ));
                    return Ok(results);
                }
            }
        }
        if !sql_server {
            self.execute("RELEASE SAVEPOINT astesia_grid_batch").await?;
        }
        Ok(results)
    }
}

pub(super) enum TransactionConnection {
    Postgres(sqlx::PgConnection),
    Mysql(sqlx::MySqlConnection),
    Sqlite(sqlx::SqliteConnection),
    SqlServer(Box<tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>>),
}

// Owning the connection prevents unfinished transactions from returning to a shared pool.
pub(super) struct OwnedTransaction(pub(super) TransactionConnection);

#[async_trait]
impl DatabaseTransaction for OwnedTransaction {
    fn db_type(&self) -> DbType {
        match &self.0 {
            TransactionConnection::Postgres(_) => DbType::PostgreSQL,
            TransactionConnection::Mysql(_) => DbType::MySQL,
            TransactionConnection::Sqlite(_) => DbType::SQLite,
            TransactionConnection::SqlServer(_) => DbType::SQLServer,
        }
    }

    async fn execute(&mut self, sql: &str) -> anyhow::Result<QueryResult> {
        match &mut self.0 {
            TransactionConnection::Postgres(connection) => {
                super::postgres::run_pg_query(connection, sql).await
            }
            TransactionConnection::Mysql(connection) => {
                super::mysql::run_mysql_query(connection, sql).await
            }
            TransactionConnection::Sqlite(connection) => {
                super::sqlite::run_sqlite_query(connection, sql).await
            }
            TransactionConnection::SqlServer(connection) => {
                super::sqlserver::run_mssql_batch(connection, sql).await
            }
        }
    }

    async fn execute_stream(
        &mut self,
        sql: &str,
        sink: &mut dyn QueryRowSink,
    ) -> anyhow::Result<QueryResult> {
        match &mut self.0 {
            TransactionConnection::Postgres(connection) => {
                super::postgres::stream_pg_query(connection, sql, sink).await
            }
            TransactionConnection::Mysql(connection) => {
                super::mysql::stream_mysql_query(connection, sql, sink).await
            }
            TransactionConnection::Sqlite(connection) => {
                super::sqlite::stream_sqlite_query(connection, sql, sink).await
            }
            TransactionConnection::SqlServer(connection) => {
                let started = std::time::Instant::now();
                let stream = connection.simple_query(sql).await?;
                super::sqlserver::consume_mssql_stream(stream, sink, started).await
            }
        }
    }

    async fn commit(mut self: Box<Self>) -> anyhow::Result<()> {
        self.execute("COMMIT").await?;
        Ok(())
    }

    async fn rollback(mut self: Box<Self>) -> anyhow::Result<()> {
        self.execute("ROLLBACK").await?;
        Ok(())
    }
}
