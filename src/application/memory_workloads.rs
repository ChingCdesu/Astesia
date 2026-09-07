use std::{path::PathBuf, time::Duration};

use super::*;
use crate::connection_repository::{SaveConnectionRequest, SharedConnectionRepository};
use crate::credential_vault::test_support::MemoryCredentialVault;
use crate::db::{ConnectionConfig, DbType, TableRef};
use crate::tasks::TaskStatus;

fn record_memory(stage: &str) {
    println!("MEMORY_STAGE {stage} pid={}", std::process::id());
    if cfg!(target_os = "macos") {
        let output = std::process::Command::new("/usr/bin/footprint")
            .args([
                "-f",
                "bytes",
                "--noCategories",
                &std::process::id().to_string(),
            ])
            .output()
            .expect("footprint measurement");
        assert!(output.status.success());
        println!("{}", String::from_utf8_lossy(&output.stdout));
    }
}

#[tokio::test]
#[ignore = "release memory workload; requires ASTESIA_MEMORY_FIXTURE_PATH and ASTESIA_MEMORY_SCENARIO"]
async fn release_memory_workload() {
    let scenario = std::env::var("ASTESIA_MEMORY_SCENARIO").expect("memory scenario");
    let fixture = PathBuf::from(std::env::var_os("ASTESIA_MEMORY_FIXTURE_PATH").expect("fixture"));
    let directory = tempfile::tempdir().unwrap();
    let repository = SharedConnectionRepository::new(
        directory.path().join("connections.sqlite3"),
        MemoryCredentialVault::shared(),
    );
    let application = Application::with_repository(repository);
    let database_path = if scenario == "restore" {
        let path = directory.path().join("restored.sqlite3");
        std::fs::File::create(&path).unwrap();
        path
    } else {
        fixture.clone()
    };
    let config = ConnectionConfig {
        id: "memory-fixture".into(),
        name: "Disposable memory fixture".into(),
        db_type: DbType::SQLite,
        host: database_path.to_string_lossy().into_owned(),
        port: 0,
        username: String::new(),
        password: String::new(),
        database: Some(database_path.to_string_lossy().into_owned()),
        color: None,
    };
    application
        .connections()
        .save_profile(ValidatedProfile::from_request(SaveConnectionRequest {
            config,
            expected_revision: None,
            mcp_enabled: false,
            group_name: None,
            tags: Vec::new(),
        }))
        .await
        .unwrap();
    let connected = application
        .connections()
        .perform_profile_operation(ProfileOperationCommand::Connect {
            connection_id: "memory-fixture".into(),
        })
        .await;
    assert!(matches!(
        connected.outcome,
        ProfileOperationOutcome::Connected(Ok(ConnectionOutcome::Succeeded))
    ));
    let loaded = application
        .connections()
        .load_databases("memory-fixture")
        .await
        .unwrap();
    let target = QueryTarget {
        connection_id: "memory-fixture".into(),
        connection_name: "Disposable memory fixture".into(),
        database: loaded.databases.first().unwrap().clone(),
        db_type: DbType::SQLite,
        session_generation: loaded.session_generation,
    };
    let output_directory = std::env::var_os("ASTESIA_MEMORY_OUTPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| directory.path().to_path_buf());
    std::fs::create_dir_all(&output_directory).unwrap();
    record_memory("ready");
    let sql = "SELECT id, value, payload FROM memory_rows ORDER BY id";
    match scenario.as_str() {
        "query" => {
            let result = application
                .queries()
                .execute(&target.connection_id, &target.database, sql)
                .await
                .unwrap();
            assert_eq!(result.rows.len(), 100_000);
            record_memory("query_retained");
            std::hint::black_box(&result);
        }
        "csv" | "json" | "xlsx" => {
            let format = match scenario.as_str() {
                "csv" => ExportFormat::Csv(CsvOptions {
                    delimiter: ",".into(),
                    include_header: true,
                    quote_all: false,
                    null_value: String::new(),
                    crlf: false,
                    bom: false,
                }),
                "json" => ExportFormat::Json(JsonOptions {
                    layout: JsonLayout::Objects,
                    pretty: false,
                }),
                _ => ExportFormat::Xlsx(XlsxOptions {
                    include_header: true,
                    sheet_name: "Rows".into(),
                }),
            };
            let output = output_directory.join(format!("rows.{scenario}"));
            let count = application
                .exports()
                .export(
                    &target.connection_id,
                    &target.database,
                    ExportSource::Sql { sql: sql.into() },
                    format,
                    output.to_string_lossy().into_owned(),
                )
                .await
                .unwrap();
            assert_eq!(count, 100_000);
            println!(
                "MEMORY_OUTPUT bytes={}",
                std::fs::metadata(output).unwrap().len()
            );
        }
        "table_chart" => {
            let table = TableRef::unqualified("memory_rows");
            let session =
                GridSession::new(target.clone(), table.clone(), DEFAULT_GRID_PAGE_SIZE).unwrap();
            let data = application
                .charts()
                .table_data(
                    target.clone(),
                    table,
                    session.query().clone(),
                    &std::sync::atomic::AtomicBool::new(false),
                )
                .await
                .unwrap()
                .expect("uncancelled table chart");
            assert_eq!(data.rows.len(), 100_000);
            let model = ChartModel::from_names(data.columns, data.rows);
            record_memory("table_chart_retained");
            std::hint::black_box(&model);
        }
        "backup" => {
            let id = application
                .transfers()
                .start_backup(
                    target.clone(),
                    BackupOptions {
                        tables: Some(vec![TableRef::unqualified("memory_rows")]),
                        content: BackupContent::StructureAndData,
                        drop_tables: DropTableMode::None,
                        output_path: output_directory
                            .join("backup.sql")
                            .to_string_lossy()
                            .into_owned(),
                    },
                )
                .await
                .unwrap();
            wait_for_task(&application, &id).await;
        }
        "restore" => {
            let id = application
                .transfers()
                .start_restore(
                    target.clone(),
                    fixture.with_extension("sql").to_string_lossy().into_owned(),
                )
                .await
                .unwrap();
            wait_for_task(&application, &id).await;
            let result = application
                .queries()
                .execute(
                    &target.connection_id,
                    &target.database,
                    "SELECT COUNT(*) FROM memory_rows",
                )
                .await
                .unwrap();
            assert_eq!(result.rows[0][0], serde_json::json!(100_000));
        }
        _ => panic!("unknown memory scenario {scenario}"),
    }
    record_memory(&format!("{scenario}_complete"));
}

async fn wait_for_task(application: &Application, id: &str) {
    tokio::time::timeout(Duration::from_secs(600), async {
        loop {
            let task = application.tasks().get_task(id).await.unwrap();
            if task.status.is_terminal() {
                assert_eq!(task.status, TaskStatus::Completed, "{}", task.message);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("memory task timeout");
}
