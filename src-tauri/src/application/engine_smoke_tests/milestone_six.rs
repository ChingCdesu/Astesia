use std::{sync::Arc, time::Duration};

use serde_json::json;

use super::{smoke_inputs, smoke_integer_type, smoke_text_type, SmokeTarget};
use crate::application::{
    Application, BackupContent, BackupOptions, ConnectionOutcome, CopyContent, CopyOptions,
    CreateObjectSpec, CsvOptions, DocumentSession, DropObjectTarget, DropTableMode, ExportFormat,
    ExportSource, JsonLayout, JsonOptions, ObjectMutation, ProfileOperationCommand,
    ProfileOperationOutcome, QueryTarget, RedisCommand, RedisListSide, RedisMutation, RedisValue,
    TableColumnSpec, ValidatedProfile, XlsxOptions,
};
use crate::connection_repository::{SaveConnectionRequest, SharedConnectionRepository};
use crate::credential_vault::test_support::MemoryCredentialVault;
use crate::db::{ConnectionConfig, DbType, SqlDialect, TableRef};
use crate::platform::ProcessSidecarHost;
use crate::tasks::{BackgroundTask, TaskStatus};

#[tokio::test]
#[ignore = "requires ASTESIA_ENGINE_SMOKE_CONFIG_PATH and seven disposable database engines"]
async fn milestone_six_data_transfer_task_and_export_workflows() {
    let (targets, target_engine) = smoke_inputs();
    let directory = tempfile::tempdir().expect("tempdir");
    let repository = SharedConnectionRepository::new(
        directory.path().join("connections.sqlite3"),
        MemoryCredentialVault::shared(),
    );
    let application = Application::with_repository(repository);

    for smoke in targets {
        let engine = smoke.config.db_type;
        if target_engine.is_some_and(|selected| selected != engine) {
            continue;
        }
        let target = connect_smoke_target(&application, &smoke, "Native milestone 6 smoke").await;
        match engine {
            DbType::MongoDB => {
                smoke_mongo_milestone_six(&application, &target, &smoke.config).await
            }
            DbType::Redis => smoke_redis_milestone_six(&application, &target).await,
            _ => smoke_sql_milestone_six(&application, &target, directory.path()).await,
        }
        smoke_export_tasks(&application, &target, directory.path()).await;
        let disconnected = application
            .connections()
            .perform_profile_operation(ProfileOperationCommand::Disconnect {
                connection_id: target.connection_id,
            })
            .await;
        assert!(matches!(
            disconnected.outcome,
            ProfileOperationOutcome::Disconnected(report) if report.is_complete()
        ));
    }
}

#[tokio::test]
#[ignore = "requires a staged debug astesia-mcp sidecar"]
async fn milestone_six_native_mcp_sidecar_lifecycle() {
    let directory = tempfile::tempdir().expect("tempdir");
    let repository = SharedConnectionRepository::new(
        directory.path().join("connections.sqlite3"),
        MemoryCredentialVault::shared(),
    );
    let application = Application::with_repository_and_sidecar(
        repository,
        Arc::new(ProcessSidecarHost::discover()),
    );
    let runtime = application.mcp().expect("MCP runtime");
    assert!(
        runtime.status().await.available,
        "debug sidecar was not discovered"
    );

    let first_port = unused_local_port();
    let first = runtime
        .start(first_port, "milestone-six-token-a".repeat(2))
        .await
        .expect("start MCP sidecar");
    assert_eq!(first.state, crate::mcp_runtime::McpServicePhase::Running);
    assert_eq!(
        first.endpoint,
        Some(format!("http://127.0.0.1:{first_port}/mcp"))
    );

    let second_port = unused_local_port();
    let second = runtime
        .restart(second_port, "milestone-six-token-b".repeat(2))
        .await
        .expect("restart MCP sidecar");
    assert_eq!(second.state, crate::mcp_runtime::McpServicePhase::Running);
    assert_eq!(
        second.endpoint,
        Some(format!("http://127.0.0.1:{second_port}/mcp"))
    );

    let stopped = runtime.stop().await.expect("stop MCP sidecar");
    assert_eq!(stopped.state, crate::mcp_runtime::McpServicePhase::Stopped);
    assert!(stopped.pid.is_none());
}

pub(super) async fn connect_smoke_target(
    application: &Application,
    smoke: &SmokeTarget,
    group: &str,
) -> QueryTarget {
    let engine = smoke.config.db_type;
    application
        .connections()
        .save_profile(ValidatedProfile::from_request(SaveConnectionRequest {
            config: smoke.config.clone(),
            expected_revision: None,
            mcp_enabled: false,
            group_name: Some(group.to_string()),
            tags: vec!["native-m6-smoke".to_string()],
        }))
        .await
        .unwrap_or_else(|error| panic!("{engine:?} configure failed: {error}"));
    let connected = application
        .connections()
        .perform_profile_operation(ProfileOperationCommand::Connect {
            connection_id: smoke.config.id.clone(),
        })
        .await;
    assert!(matches!(
        connected.outcome,
        ProfileOperationOutcome::Connected(Ok(ConnectionOutcome::Succeeded))
    ));
    let loaded = application
        .connections()
        .load_databases(&smoke.config.id)
        .await
        .unwrap_or_else(|error| panic!("{engine:?} database load failed: {error}"));
    let database = smoke
        .browse_database
        .clone()
        .or_else(|| smoke.config.database.clone())
        .or_else(|| loaded.databases.first().cloned())
        .unwrap_or_else(|| panic!("{engine:?} returned no browseable database"));
    QueryTarget {
        connection_id: smoke.config.id.clone(),
        connection_name: smoke.config.name.clone(),
        database,
        db_type: engine,
        session_generation: loaded.session_generation,
    }
}

async fn smoke_mongo_milestone_six(
    application: &Application,
    target: &QueryTarget,
    config: &ConnectionConfig,
) {
    use mongodb::{
        bson::{doc, Document},
        options::{ClientOptions, Credential, ServerAddress},
        Client,
    };

    let credential = (!config.username.is_empty()).then(|| {
        Credential::builder()
            .username(config.username.clone())
            .password(config.password.clone())
            .build()
    });
    let options = ClientOptions::builder()
        .hosts(vec![ServerAddress::Tcp {
            host: config.host.clone(),
            port: Some(config.port),
        }])
        .credential(credential)
        .build();
    let client = Client::with_options(options).expect("Mongo client");
    let collection = client
        .database(&target.database)
        .collection::<Document>("astesia_m6_documents");
    collection
        .delete_many(doc! {})
        .await
        .expect("clear Mongo fixture");
    collection
        .insert_many([
            doc! { "kind": "match", "ordinal": 1, "nested": { "active": true } },
            doc! { "kind": "match", "ordinal": 2, "nested": { "active": false } },
            doc! { "kind": "other", "ordinal": 3 },
        ])
        .await
        .expect("seed Mongo fixture");

    let mut session = DocumentSession::new(
        target.clone(),
        TableRef::unqualified("astesia_m6_documents"),
        1,
    )
    .expect("Mongo document session");
    session
        .set_filter("{\"kind\":\"match\"}".to_string())
        .expect("Mongo filter");
    let first_request = session.begin_load().expect("first Mongo page request");
    let first = application
        .documents()
        .load(&first_request)
        .await
        .expect("first Mongo page");
    assert_eq!(first.total_documents, 2);
    assert_eq!(first.documents.len(), 1);
    assert!(session.finish_load(&first_request, Ok(first)));
    session.set_page(2).expect("second Mongo page");
    let second_request = session.begin_load().expect("second Mongo page request");
    let second = application
        .documents()
        .load(&second_request)
        .await
        .expect("second Mongo page");
    assert_eq!(second.documents.len(), 1);
    assert!(second.documents[0].get("nested").is_some());
    assert!(session.finish_load(&second_request, Ok(second)));
    collection
        .delete_many(doc! {})
        .await
        .expect("remove Mongo fixture");
}

async fn smoke_redis_milestone_six(application: &Application, target: &QueryTarget) {
    let prefix = "astesia:m6";
    let keys = ["string", "hash", "list", "set", "zset"].map(|suffix| format!("{prefix}:{suffix}"));
    for key in &keys {
        let _ = application
            .redis()
            .mutate(target, key, RedisMutation::Delete)
            .await;
    }
    application
        .redis()
        .mutate(
            target,
            &keys[0],
            RedisMutation::SetString {
                value: "hello world".to_string(),
                ttl_seconds: Some(120),
            },
        )
        .await
        .expect("set Redis string");
    application
        .redis()
        .mutate(
            target,
            &keys[1],
            RedisMutation::HashSet {
                field: "field".to_string(),
                value: "value".to_string(),
            },
        )
        .await
        .expect("set Redis hash field");
    application
        .redis()
        .mutate(
            target,
            &keys[2],
            RedisMutation::ListPush {
                side: RedisListSide::Right,
                value: "item".to_string(),
            },
        )
        .await
        .expect("push Redis list item");
    application
        .redis()
        .mutate(
            target,
            &keys[3],
            RedisMutation::SetAdd {
                member: "member".to_string(),
            },
        )
        .await
        .expect("add Redis set member");
    application
        .redis()
        .mutate(
            target,
            &keys[4],
            RedisMutation::SortedSetAdd {
                member: "member".to_string(),
                score: 1.5,
            },
        )
        .await
        .expect("add Redis sorted-set member");

    let scanned = application
        .redis()
        .scan_keys(target, prefix)
        .await
        .expect("SCAN Redis fixtures");
    assert_eq!(scanned.len(), keys.len());
    assert!(matches!(
        application.redis().key(target, &keys[0]).await.unwrap().value,
        RedisValue::String(ref value) if value == "hello world"
    ));
    assert!(matches!(
        application.redis().key(target, &keys[1]).await.unwrap().value,
        RedisValue::Hash(ref values) if values == &[("field".to_string(), "value".to_string())]
    ));
    assert!(matches!(
        application.redis().key(target, &keys[2]).await.unwrap().value,
        RedisValue::List(ref values) if values == &["item".to_string()]
    ));
    assert!(matches!(
        application.redis().key(target, &keys[3]).await.unwrap().value,
        RedisValue::Set(ref values) if values == &["member".to_string()]
    ));
    assert!(matches!(
        application.redis().key(target, &keys[4]).await.unwrap().value,
        RedisValue::SortedSet(ref values) if values == &[("member".to_string(), 1.5)]
    ));
    let raw = application
        .redis()
        .execute(
            target,
            RedisCommand::parse(&format!("GET '{}'", keys[0])).expect("raw Redis command"),
        )
        .await
        .expect("execute raw Redis command");
    assert_eq!(raw.rows[0][0], json!("hello world"));

    for key in &keys {
        application
            .redis()
            .mutate(target, key, RedisMutation::Delete)
            .await
            .expect("delete Redis fixture");
        assert!(matches!(
            application.redis().key(target, key).await.unwrap().value,
            RedisValue::Missing
        ));
    }
}

async fn smoke_sql_milestone_six(
    application: &Application,
    target: &QueryTarget,
    directory: &std::path::Path,
) {
    let engine = target.db_type;
    let source_name = "astesia_m6_transfer";
    let copy_name = "astesia_m6_transfer_copy";
    let source_table = TableRef::unqualified(source_name);
    for name in [copy_name, source_name] {
        let _ = application
            .objects()
            .execute(
                target,
                &ObjectMutation::Drop(DropObjectTarget::Table(name.to_string())),
            )
            .await;
    }
    application
        .objects()
        .execute(
            target,
            &ObjectMutation::Create(CreateObjectSpec::Table {
                name: source_name.to_string(),
                columns: vec![
                    TableColumnSpec {
                        name: "id".to_string(),
                        data_type: smoke_integer_type(engine).to_string(),
                        nullable: false,
                        primary_key: true,
                        default_value: None,
                    },
                    TableColumnSpec {
                        name: "label".to_string(),
                        data_type: smoke_text_type(engine).to_string(),
                        nullable: false,
                        primary_key: false,
                        default_value: None,
                    },
                ],
            }),
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} M6 table creation failed: {error}"));
    let dialect = SqlDialect::new(engine);
    let quoted = dialect.quote_table_ref(&source_table).unwrap();
    application
        .queries()
        .execute(
            &target.connection_id,
            &target.database,
            &format!(
                "INSERT INTO {quoted} ({}, {}) VALUES (1, 'one'), (2, 'two')",
                dialect.quote_identifier("id").unwrap(),
                dialect.quote_identifier("label").unwrap()
            ),
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} M6 seed failed: {error}"));

    let backup_path = directory.join(format!("{engine:?}-m6-backup.sql"));
    let backup_id = application
        .transfers()
        .start_backup(
            target.clone(),
            BackupOptions {
                tables: Some(vec![source_table.clone()]),
                content: BackupContent::StructureAndData,
                drop_tables: DropTableMode::DropIfExists,
                output_path: backup_path.to_string_lossy().into_owned(),
            },
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} backup start failed: {error}"));
    assert_task_completed(application, &backup_id, engine, "backup").await;
    assert!(backup_path.is_file(), "{engine:?} backup file missing");

    let copy_id = application
        .transfers()
        .start_table_copy(
            target.clone(),
            source_table.clone(),
            target.clone(),
            CopyOptions {
                content: CopyContent::StructureAndData,
                new_table_name: copy_name.to_string(),
            },
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} copy start failed: {error}"));
    assert_task_completed(application, &copy_id, engine, "copy").await;

    application
        .objects()
        .execute(
            target,
            &ObjectMutation::Drop(DropObjectTarget::Table(source_name.to_string())),
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} pre-restore drop failed: {error}"));
    let restore_id = application
        .transfers()
        .start_restore(target.clone(), backup_path.to_string_lossy().into_owned())
        .await
        .unwrap_or_else(|error| panic!("{engine:?} restore start failed: {error}"));
    assert_task_completed(application, &restore_id, engine, "restore").await;
    let restored = application
        .queries()
        .execute(
            &target.connection_id,
            &target.database,
            &format!("SELECT COUNT(*) AS count FROM {quoted}"),
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} restored table query failed: {error}"));
    let restored_count = restored.rows[0][0]
        .as_u64()
        .or_else(|| {
            restored.rows[0][0]
                .as_i64()
                .and_then(|value| u64::try_from(value).ok())
        })
        .or_else(|| {
            restored.rows[0][0]
                .as_str()
                .and_then(|value| value.parse().ok())
        });
    assert_eq!(restored_count, Some(2));

    for name in [copy_name, source_name] {
        application
            .objects()
            .execute(
                target,
                &ObjectMutation::Drop(DropObjectTarget::Table(name.to_string())),
            )
            .await
            .unwrap_or_else(|error| panic!("{engine:?} M6 cleanup failed: {error}"));
    }
}

async fn smoke_export_tasks(
    application: &Application,
    target: &QueryTarget,
    directory: &std::path::Path,
) {
    let formats = [
        (
            "csv",
            ExportFormat::Csv(CsvOptions {
                delimiter: ",".to_string(),
                include_header: true,
                quote_all: false,
                null_value: "\\N".to_string(),
                crlf: false,
                bom: false,
            }),
        ),
        (
            "json",
            ExportFormat::Json(JsonOptions {
                layout: JsonLayout::Objects,
                pretty: true,
            }),
        ),
        (
            "xlsx",
            ExportFormat::Xlsx(XlsxOptions {
                include_header: true,
                sheet_name: "Milestone 6".to_string(),
            }),
        ),
    ];
    for (extension, format) in formats {
        let path = directory.join(format!("{:?}-m6.{extension}", target.db_type));
        let id = application
            .exports()
            .start_export(
                target.clone(),
                ExportSource::Rows {
                    columns: vec!["id".to_string(), "label".to_string()],
                    rows: vec![vec![json!(1), json!("one")], vec![json!(2), json!("two")]],
                },
                format,
                path.to_string_lossy().into_owned(),
            )
            .await
            .expect("start export task");
        let task = assert_task_completed(application, &id, target.db_type, extension).await;
        assert!(task.message.contains("2 row(s)"));
        assert!(
            path.is_file(),
            "{:?} {extension} export missing",
            target.db_type
        );
    }
}

async fn assert_task_completed(
    application: &Application,
    id: &str,
    engine: DbType,
    operation: &str,
) -> BackgroundTask {
    for _ in 0..400 {
        let task = application.tasks().get_task(id).await.expect("task exists");
        if matches!(
            task.status,
            TaskStatus::Completed
                | TaskStatus::Partial
                | TaskStatus::Failed
                | TaskStatus::Cancelled
        ) {
            assert_eq!(
                task.status,
                TaskStatus::Completed,
                "{engine:?} {operation} did not complete: {}",
                task.message
            );
            assert_eq!(task.progress, 1.0);
            return task;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("{engine:?} {operation} task timed out");
}

fn unused_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral port")
        .local_addr()
        .expect("read ephemeral port")
        .port()
}
