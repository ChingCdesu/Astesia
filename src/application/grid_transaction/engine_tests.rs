use super::*;
use crate::db::{create_driver, ConnectionConfig, DbType, TransactionIsolation};

async fn exercise(engine: DbType, port: u16, username: &str, password: String) {
    let mut driver = create_driver(&ConnectionConfig {
        id: "figma-transaction-fixture".into(),
        name: "Figma transaction fixture".into(),
        db_type: engine,
        host: "127.0.0.1".into(),
        port,
        username: username.into(),
        password,
        database: (engine != DbType::SQLServer).then(|| "astesia_figma".into()),
        color: None,
    });
    driver.connect().await.unwrap();
    if engine == DbType::SQLServer {
        driver
            .execute_query(
                "master",
                "IF DB_ID('astesia_figma') IS NULL CREATE DATABASE astesia_figma",
            )
            .await
            .unwrap();
    }
    driver
        .execute_query(
            "astesia_figma",
            "DROP TABLE IF EXISTS astesia_transaction_fixture",
        )
        .await
        .unwrap();
    driver
        .execute_query(
            "astesia_figma",
            "CREATE TABLE astesia_transaction_fixture (id INT PRIMARY KEY)",
        )
        .await
        .unwrap();
    let target = QueryTarget {
        connection_id: "figma-transaction-fixture".into(),
        connection_name: "Figma transaction fixture".into(),
        database: "astesia_figma".into(),
        db_type: engine,
        session_generation: 1,
    };
    for isolation in engine.transaction_isolations() {
        driver
            .execute_query("astesia_figma", "DELETE FROM astesia_transaction_fixture")
            .await
            .unwrap();
        let (_retirement, retired) = watch::channel(false);
        let transaction = GridTransaction::start(
            target.clone(),
            driver
                .begin_transaction("astesia_figma", *isolation)
                .await
                .unwrap(),
            retired.clone(),
        );
        transaction
            .apply(vec![
                "INSERT INTO astesia_transaction_fixture VALUES (1)".into()
            ])
            .await
            .unwrap();
        let failed = transaction
            .apply(vec![
                "INSERT INTO astesia_transaction_fixture VALUES (2)".into(),
                "INSERT INTO astesia_transaction_fixture VALUES (1)".into(),
            ])
            .await
            .unwrap();
        assert!(!failed.last().unwrap().success, "{engine:?} {isolation:?}");
        assert_eq!(
            transaction
                .query("SELECT COUNT(*) FROM astesia_transaction_fixture".into())
                .await
                .unwrap()
                .rows[0][0],
            1
        );
        let outside_sql = if engine == DbType::SQLServer {
            "SELECT COUNT(*) FROM astesia_transaction_fixture WITH (READPAST)"
        } else {
            "SELECT COUNT(*) FROM astesia_transaction_fixture"
        };
        assert_eq!(
            driver
                .execute_query("astesia_figma", outside_sql)
                .await
                .unwrap()
                .rows[0][0],
            0
        );
        let mut streamed = crate::db::QueryRowCollector::new(None);
        transaction
            .stream_rows(
                "SELECT id FROM astesia_transaction_fixture".into(),
                &mut streamed,
            )
            .await
            .unwrap();
        let streamed = streamed.finish(crate::db::QueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            affected_rows: 0,
            execution_time_ms: 0,
        });
        assert_eq!(streamed.rows, vec![vec![serde_json::json!(1)]]);
        assert!(transaction
            .stream_rows(
                "SELECT missing FROM astesia_transaction_fixture".into(),
                &mut crate::db::QueryRowCollector::new(None)
            )
            .await
            .is_err());
        assert!(!transaction.is_closed());
        if engine == DbType::PostgreSQL {
            let level = transaction
                .query("SHOW transaction_isolation".into())
                .await
                .unwrap();
            if let Some(expected) = isolation.sql() {
                assert_eq!(level.rows[0][0].as_str().unwrap().to_uppercase(), expected);
            }
        }
        if engine == DbType::SQLServer {
            let level = transaction.query("SELECT transaction_isolation_level FROM sys.dm_exec_sessions WHERE session_id = @@SPID".into()).await.unwrap();
            let expected = match isolation {
                TransactionIsolation::ReadCommitted | TransactionIsolation::DatabaseDefault => 2,
                TransactionIsolation::RepeatableRead => 3,
                TransactionIsolation::Serializable => 4,
            };
            assert_eq!(level.rows[0][0], expected);
        }
        transaction.finish(true).await.unwrap();
        assert_eq!(
            driver
                .execute_query(
                    "astesia_figma",
                    "SELECT COUNT(*) FROM astesia_transaction_fixture"
                )
                .await
                .unwrap()
                .rows[0][0],
            1
        );
        let rollback = GridTransaction::start(
            target.clone(),
            driver
                .begin_transaction("astesia_figma", *isolation)
                .await
                .unwrap(),
            retired,
        );
        rollback
            .apply(vec![
                "INSERT INTO astesia_transaction_fixture VALUES (3)".into()
            ])
            .await
            .unwrap();
        rollback.finish(false).await.unwrap();
        assert_eq!(
            driver
                .execute_query(
                    "astesia_figma",
                    "SELECT COUNT(*) FROM astesia_transaction_fixture"
                )
                .await
                .unwrap()
                .rows[0][0],
            1
        );
    }
    driver
        .execute_query("astesia_figma", "DROP TABLE astesia_transaction_fixture")
        .await
        .unwrap();
    driver.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture on localhost:55432"]
async fn figma_postgres_transactions() {
    exercise(DbType::PostgreSQL, 55432, "postgres", String::new()).await;
}

#[tokio::test]
#[ignore = "requires isolated MySQL fixture on localhost:53306"]
async fn figma_mysql_transactions() {
    exercise(DbType::MySQL, 53306, "root", String::new()).await;
}

#[tokio::test]
#[ignore = "requires isolated SQL Server fixture on localhost:51433 and ASTESIA_TEST_MSSQL_PASSWORD"]
async fn figma_sqlserver_transactions() {
    exercise(
        DbType::SQLServer,
        51433,
        "sa",
        std::env::var("ASTESIA_TEST_MSSQL_PASSWORD").expect("fixture password"),
    )
    .await;
}
