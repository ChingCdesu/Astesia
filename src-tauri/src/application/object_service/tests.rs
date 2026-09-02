use super::*;
use crate::application::ConnectionOutcome;
use crate::connection_repository::SharedConnectionRepository;
use crate::credential_vault::test_support::MemoryCredentialVault;
use crate::db::ConnectionConfig;

fn column(name: &str, data_type: &str, nullable: bool, primary_key: bool) -> TableColumnSpec {
    TableColumnSpec {
        name: name.to_string(),
        data_type: data_type.to_string(),
        nullable,
        primary_key,
        default_value: None,
    }
}

#[test]
fn create_ddl_is_engine_specific_and_capability_gated() {
    assert_eq!(
            render_object_mutation(
                DbType::PostgreSQL,
                &ObjectMutation::Create(CreateObjectSpec::Table {
                    name: "audit.events".to_string(),
                    columns: vec![
                        column("tenant", "uuid", false, true),
                        column("id", "bigint", false, true),
                        column("payload", "jsonb", true, false),
                    ],
                }),
            )
            .unwrap(),
            "CREATE TABLE \"audit\".\"events\" (\n  \"tenant\" uuid,\n  \"id\" bigint,\n  \"payload\" jsonb,\n  PRIMARY KEY (\"tenant\", \"id\")\n)"
        );
    assert_eq!(
            render_object_mutation(
                DbType::ClickHouse,
                &ObjectMutation::Create(CreateObjectSpec::Table {
                    name: "events".to_string(),
                    columns: vec![
                        column("id", "UInt64", false, true),
                        column("label", "LowCardinality(String)", true, false),
                    ],
                }),
            )
            .unwrap(),
            "CREATE TABLE `events` (\n  `id` UInt64,\n  `label` LowCardinality(Nullable(String))\n)\nENGINE = MergeTree\nORDER BY (`id`)"
        );
    assert!(render_object_mutation(
        DbType::SQLite,
        &ObjectMutation::Create(CreateObjectSpec::Procedure {
            name: "refresh".to_string(),
            arguments: String::new(),
            language: "sql".to_string(),
            body: "SELECT 1".to_string(),
        })
    )
    .is_err());
}

#[test]
fn routines_triggers_and_users_preserve_engine_syntax() {
    let function = ObjectMutation::Create(CreateObjectSpec::Function {
        name: "billing.total".to_string(),
        arguments: "account_id uuid".to_string(),
        return_type: "numeric".to_string(),
        language: "plpgsql".to_string(),
        body: "BEGIN\nRETURN 0;\nEND;".to_string(),
    });
    let sql = render_object_mutation(DbType::PostgreSQL, &function).unwrap();
    assert!(sql.starts_with("CREATE FUNCTION \"billing\".\"total\"(account_id uuid)"));
    assert!(sql.contains("$astesia$\nBEGIN\nRETURN 0;\nEND;\n$astesia$"));

    let trigger = ObjectMutation::Create(CreateObjectSpec::Trigger {
        name: "audit_users".to_string(),
        table: TableRef::qualified("public", "users"),
        timing: TriggerTiming::After,
        event: TriggerEvent::Update,
        body: "INSERT INTO audit_log(id) VALUES (NEW.id);".to_string(),
    });
    assert_eq!(
            render_object_mutation(DbType::SQLite, &trigger).unwrap(),
            "CREATE TRIGGER \"audit_users\"\nAFTER UPDATE ON \"public\".\"users\"\nBEGIN\nINSERT INTO audit_log(id) VALUES (NEW.id);\nEND"
        );

    let user = ObjectMutation::Create(CreateObjectSpec::User {
        name: "operator".to_string(),
        host: Some("localhost".to_string()),
        password: "it's safe".to_string(),
    });
    assert_eq!(
        render_object_mutation(DbType::MySQL, &user).unwrap(),
        "CREATE USER 'operator'@'localhost' IDENTIFIED BY 'it''s safe'"
    );
}

#[test]
fn rename_and_drop_quote_names_and_preserve_routine_signatures() {
    assert_eq!(
        render_object_mutation(
            DbType::PostgreSQL,
            &ObjectMutation::Rename {
                kind: DatabaseObjectKind::Table,
                name: "audit.events".to_string(),
                new_name: "events_archive".to_string(),
            },
        )
        .unwrap(),
        "ALTER TABLE \"audit\".\"events\" RENAME TO \"events_archive\""
    );
    assert_eq!(
        render_object_mutation(
            DbType::PostgreSQL,
            &ObjectMutation::Drop(DropObjectTarget::Function(
                "billing.total(uuid, integer)".to_string(),
            )),
        )
        .unwrap(),
        "DROP FUNCTION \"billing\".\"total\"(uuid, integer)"
    );
    assert_eq!(
        render_object_mutation(
            DbType::PostgreSQL,
            &ObjectMutation::Drop(DropObjectTarget::Trigger {
                name: "public.audit_users".to_string(),
                table: "public.users".to_string(),
            }),
        )
        .unwrap(),
        "DROP TRIGGER \"audit_users\" ON \"public\".\"users\""
    );
}

#[test]
fn fragments_reject_statement_breakout_but_bodies_allow_real_sql() {
    let invalid = ObjectMutation::Create(CreateObjectSpec::Table {
        name: "users".to_string(),
        columns: vec![column("id", "integer; DROP TABLE users", false, true)],
    });
    assert!(matches!(
        render_object_mutation(DbType::SQLite, &invalid),
        Err(ObjectMutationError::Invalid(_))
    ));
    assert_eq!(
        dollar_quote("SELECT '$astesia$';"),
        "$astesia_$\nSELECT '$astesia$';\n$astesia_$"
    );
    let unsupported_nullable = ObjectMutation::Create(CreateObjectSpec::Table {
        name: "events".to_string(),
        columns: vec![column("tags", "Array(String)", true, false)],
    });
    assert!(matches!(
        render_object_mutation(DbType::ClickHouse, &unsupported_nullable),
        Err(ObjectMutationError::Invalid(_))
    ));
}

#[test]
fn display_identities_never_include_credentials() {
    let mutation = ObjectMutation::Create(CreateObjectSpec::User {
        name: "operator".to_string(),
        host: Some("localhost".to_string()),
        password: "correct horse battery staple".to_string(),
    });
    assert_eq!(mutation.display_identity(), "operator@localhost");
    assert!(!mutation
        .display_identity()
        .contains("correct horse battery staple"));

    assert_eq!(
        render_object_mutation(
            DbType::SQLServer,
            &ObjectMutation::Rename {
                kind: DatabaseObjectKind::Table,
                name: "audit.event]log".to_string(),
                new_name: "event]archive".to_string(),
            },
        )
        .unwrap(),
        "EXEC sp_rename N'[audit].[event]]log]', N'event]archive', 'OBJECT'"
    );
}

#[tokio::test]
async fn service_executes_create_rename_and_drop_on_one_live_session() {
    let directory = tempfile::TempDir::new().unwrap();
    let database_path = directory.path().join("objects.sqlite3");
    std::fs::File::create(&database_path).unwrap();
    let repository = SharedConnectionRepository::new(
        directory.path().join("connections.sqlite3"),
        MemoryCredentialVault::shared(),
    );
    repository
        .create(
            ConnectionConfig {
                id: "local".to_string(),
                name: "Local".to_string(),
                db_type: DbType::SQLite,
                host: database_path.display().to_string(),
                port: 0,
                username: String::new(),
                password: String::new(),
                database: None,
                color: None,
            },
            false,
        )
        .await
        .unwrap();
    let manager = ConnectionManager::new(repository);
    assert_eq!(
        manager.connect("local").await.unwrap(),
        ConnectionOutcome::Succeeded
    );
    let (_, session_generation) = manager.driver_session("local").await.unwrap();
    let target = QueryTarget {
        connection_id: "local".to_string(),
        connection_name: "Local".to_string(),
        database: "main".to_string(),
        db_type: DbType::SQLite,
        session_generation,
    };
    let service = ObjectService::new(manager.clone());
    service
        .execute(
            &target,
            &ObjectMutation::Create(CreateObjectSpec::Table {
                name: "events".to_string(),
                columns: vec![column("id", "INTEGER", false, true)],
            }),
        )
        .await
        .unwrap();
    service
        .execute(
            &target,
            &ObjectMutation::Rename {
                kind: DatabaseObjectKind::Table,
                name: "events".to_string(),
                new_name: "events_archive".to_string(),
            },
        )
        .await
        .unwrap();
    service
        .execute(
            &target,
            &ObjectMutation::Drop(DropObjectTarget::Table("events_archive".to_string())),
        )
        .await
        .unwrap();

    let handle = manager.driver("local").await.unwrap();
    let driver = handle.lock_active().await.unwrap();
    assert!(driver.get_tables("main").await.unwrap().is_empty());
}
