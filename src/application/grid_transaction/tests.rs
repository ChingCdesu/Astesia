use super::*;
use crate::db::{create_driver, ConnectionConfig, DbType, TransactionIsolation};

#[tokio::test]
async fn manual_batches_commit_rollback_and_retirement_are_isolated() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::File::create(directory.path().join("database.sqlite")).unwrap();
    let config = ConnectionConfig {
        id: "transaction-test".into(),
        name: "Transaction test".into(),
        db_type: DbType::SQLite,
        host: directory
            .path()
            .join("database.sqlite")
            .to_string_lossy()
            .into_owned(),
        port: 0,
        username: String::new(),
        password: String::new(),
        database: None,
        color: None,
    };
    let mut driver = create_driver(&config);
    driver.connect().await.unwrap();
    driver
        .execute_query("main", "CREATE TABLE items (id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    let target = QueryTarget {
        connection_id: config.id.clone(),
        connection_name: config.name.clone(),
        database: "main".into(),
        db_type: DbType::SQLite,
        session_generation: 1,
    };
    let (retirement, retired) = watch::channel(false);
    let transaction = GridTransaction::start(
        target.clone(),
        driver
            .begin_transaction("main", TransactionIsolation::Serializable)
            .await
            .unwrap(),
        retired.clone(),
    );
    assert!(
        transaction
            .apply(vec!["INSERT INTO items VALUES (1)".into()])
            .await
            .unwrap()[0]
            .success
    );
    let failed = transaction
        .apply(vec![
            "INSERT INTO items VALUES (2)".into(),
            "INSERT INTO items VALUES (1)".into(),
        ])
        .await
        .unwrap();
    assert!(!failed.last().unwrap().success);
    assert_eq!(
        transaction
            .query("SELECT COUNT(*) AS n FROM items".into())
            .await
            .unwrap()
            .rows[0][0],
        1
    );
    assert_eq!(
        driver
            .execute_query("main", "SELECT COUNT(*) AS n FROM items")
            .await
            .unwrap()
            .rows[0][0],
        0
    );
    assert_streaming_export(&transaction, directory.path()).await;
    let mut sink = LimitedSink::default();
    transaction.stream_rows("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000) SELECT x FROM n".into(), &mut sink).await.unwrap();
    assert_eq!(sink.rows.len(), 1);
    assert!(transaction
        .stream_rows(
            "SELECT missing FROM items".into(),
            &mut LimitedSink::default()
        )
        .await
        .is_err());
    assert!(!transaction.is_closed());
    assert!(transaction
        .query("SELECT missing FROM items".into())
        .await
        .is_err());
    assert_eq!(
        transaction
            .query("SELECT COUNT(*) AS n FROM items".into())
            .await
            .unwrap()
            .rows[0][0],
        1
    );
    transaction.finish(true).await.unwrap();
    assert_eq!(
        driver
            .execute_query("main", "SELECT COUNT(*) AS n FROM items")
            .await
            .unwrap()
            .rows[0][0],
        1
    );

    let transaction = GridTransaction::start(
        target.clone(),
        driver
            .begin_transaction("main", TransactionIsolation::DatabaseDefault)
            .await
            .unwrap(),
        retired.clone(),
    );
    transaction
        .apply(vec!["INSERT INTO items VALUES (3)".into()])
        .await
        .unwrap();
    transaction.finish(false).await.unwrap();
    assert_eq!(
        driver
            .execute_query("main", "SELECT COUNT(*) AS n FROM items")
            .await
            .unwrap()
            .rows[0][0],
        1
    );

    let transaction = GridTransaction::start(
        target,
        driver
            .begin_transaction("main", TransactionIsolation::DatabaseDefault)
            .await
            .unwrap(),
        retired,
    );
    transaction
        .apply(vec!["INSERT INTO items VALUES (4)".into()])
        .await
        .unwrap();
    retirement.send_replace(true);
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        transaction.commands.closed(),
    )
    .await
    .unwrap();
    assert!(transaction.is_closed());
    assert!(transaction.has_pending_changes());
    assert!(transaction
        .recovery_sql()
        .contains("INSERT INTO items VALUES (4)"));
    assert!(transaction.finish(true).await.is_err());
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        driver.execute_query("main", "INSERT INTO items VALUES (4)"),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        driver
            .execute_query("main", "SELECT COUNT(*) AS n FROM items")
            .await
            .unwrap()
            .rows[0][0],
        2
    );
}

#[derive(Default)]
struct LimitedSink {
    rows: Vec<Vec<serde_json::Value>>,
}

#[async_trait::async_trait]
impl crate::db::QueryRowSink for LimitedSink {
    fn wants_rows(&self) -> bool {
        self.rows.is_empty()
    }
    async fn row(&mut self, row: Vec<serde_json::Value>) {
        self.rows.push(row);
    }
}

async fn assert_streaming_export(transaction: &GridTransaction, directory: &std::path::Path) {
    use crate::application::{Application, CsvOptions, ExportFormat, ExportSource};
    use crate::connection_repository::SharedConnectionRepository;
    use crate::credential_vault::test_support::MemoryCredentialVault;
    use crate::tasks::TaskStatus;

    let application = Application::with_repository(SharedConnectionRepository::new(
        directory.join("profiles.sqlite"),
        MemoryCredentialVault::shared(),
    ));
    let output = directory.join("uncommitted.csv");
    let id = application
        .exports()
        .start_transaction_export(
            transaction.clone(),
            ExportSource::Sql {
                sql: "SELECT id FROM items ORDER BY id".into(),
            },
            ExportFormat::Csv(CsvOptions {
                delimiter: ",".into(),
                include_header: true,
                quote_all: false,
                null_value: "NULL".into(),
                crlf: false,
                bom: false,
            }),
            output.to_string_lossy().into_owned(),
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let task = application.tasks().get_task(&id).await.unwrap();
            match task.status {
                TaskStatus::Completed => break,
                TaskStatus::Failed | TaskStatus::Partial | TaskStatus::Cancelled => {
                    panic!("export failed: {}", task.message)
                }
                _ => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("transaction export completes");
    assert_eq!(std::fs::read_to_string(output).unwrap(), "id\n1\n");
    assert!(transaction.has_pending_changes());
}

#[test]
fn isolation_choices_follow_the_database_engine() {
    assert!(DbType::SQLite
        .transaction_isolations()
        .contains(&TransactionIsolation::Serializable));
    assert!(!DbType::SQLite
        .transaction_isolations()
        .contains(&TransactionIsolation::ReadCommitted));
    for engine in [DbType::MySQL, DbType::PostgreSQL, DbType::SQLServer] {
        assert!(engine
            .transaction_isolations()
            .contains(&TransactionIsolation::RepeatableRead));
    }
    for engine in [DbType::Redis, DbType::MongoDB, DbType::ClickHouse] {
        assert!(engine.transaction_isolations().is_empty());
    }
}
