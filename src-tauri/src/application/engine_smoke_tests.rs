use std::collections::HashSet;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::json;

use super::{
    Application, CatalogSection, ConnectionOutcome, CreateObjectSpec, CsvOptions,
    DatabaseObjectKind, DropObjectTarget, ExportFormat, ExportSource, GridCell, GridEditability,
    GridRowSelectionMode, GridSession, GridSessionError, GridSort, GridSortDirection,
    ObjectMutation, ProfileOperationCommand, ProfileOperationOutcome, QueryTarget, TableColumnSpec,
    TriggerEvent, TriggerTiming, ValidatedProfile,
};
use crate::connection_repository::{SaveConnectionRequest, SharedConnectionRepository};
use crate::credential_vault::test_support::MemoryCredentialVault;
use crate::db::{ConnectionConfig, DbType, SqlDialect, TableRef};

mod milestone_six;

#[derive(Deserialize)]
struct SmokeTarget {
    config: ConnectionConfig,
    browse_database: Option<String>,
}

fn smoke_inputs() -> (Vec<SmokeTarget>, Option<DbType>) {
    let path = std::env::var_os("ASTESIA_ENGINE_SMOKE_CONFIG_PATH")
        .map(PathBuf::from)
        .expect("ASTESIA_ENGINE_SMOKE_CONFIG_PATH is required");
    let targets: Vec<SmokeTarget> =
        serde_json::from_slice(&std::fs::read(path).expect("read engine smoke configuration"))
            .expect("parse engine smoke configuration");
    let engines = targets
        .iter()
        .map(|target| target.config.db_type)
        .collect::<HashSet<_>>();
    assert_eq!(engines, HashSet::from(DbType::all()));
    let target_engine = std::env::var("ASTESIA_ENGINE_SMOKE_TARGET")
        .ok()
        .map(|value| {
            serde_json::from_value::<DbType>(serde_json::Value::String(value))
                .expect("ASTESIA_ENGINE_SMOKE_TARGET must name a supported engine")
        });
    (targets, target_engine)
}

#[tokio::test]
#[ignore = "requires ASTESIA_ENGINE_SMOKE_CONFIG_PATH and seven disposable database engines"]
async fn all_engines_cross_the_application_connection_workflow() {
    let (targets, target_engine) = smoke_inputs();

    let directory = tempfile::tempdir().expect("tempdir");
    let repository = SharedConnectionRepository::new(
        directory.path().join("connections.sqlite3"),
        MemoryCredentialVault::shared(),
    );
    let application = Application::with_repository(repository);

    for target in targets {
        let engine = target.config.db_type;
        if target_engine.is_some_and(|selected| selected != engine) {
            continue;
        }
        let connection_id = target.config.id.clone();
        let profile = ValidatedProfile::from_request(SaveConnectionRequest {
            config: target.config.clone(),
            expected_revision: None,
            mcp_enabled: false,
            group_name: Some("Native smoke".to_string()),
            tags: vec!["native-smoke".to_string()],
        });
        application
            .connections()
            .save_profile(profile)
            .await
            .unwrap_or_else(|error| panic!("{engine:?} configure failed: {error}"));

        let tested = application
            .connections()
            .test_connection(target.config.clone())
            .await
            .unwrap_or_else(|error| panic!("{engine:?} test failed: {error}"));
        assert_eq!(
            tested,
            ConnectionOutcome::Succeeded,
            "{engine:?} connection test was rejected"
        );

        let connected = application
            .connections()
            .perform_profile_operation(ProfileOperationCommand::Connect {
                connection_id: connection_id.clone(),
            })
            .await;
        assert!(matches!(
            connected.outcome,
            ProfileOperationOutcome::Connected(Ok(ConnectionOutcome::Succeeded))
        ));

        let databases = application
            .connections()
            .load_databases(&connection_id)
            .await
            .unwrap_or_else(|error| panic!("{engine:?} database load failed: {error}"));
        let database = target
            .browse_database
            .or_else(|| target.config.database.clone())
            .or_else(|| databases.databases.first().cloned())
            .unwrap_or_else(|| panic!("{engine:?} returned no browseable database"));
        application
            .catalog()
            .tables(&connection_id, &database)
            .await
            .unwrap_or_else(|error| panic!("{engine:?} browse failed: {error}"));

        if engine.capabilities().sql {
            let results = application
                .queries()
                .execute_statements(
                    &connection_id,
                    &database,
                    vec!["SELECT 1 AS astesia_milestone_4".to_string()],
                )
                .await
                .unwrap_or_else(|error| panic!("{engine:?} query failed: {error}"));
            assert_eq!(results.len(), 1, "{engine:?} returned extra results");
            assert!(results[0].success, "{engine:?} query was unsuccessful");
            assert_eq!(results[0].columns.len(), 1, "{engine:?} lost the column");
            assert_eq!(results[0].rows.len(), 1, "{engine:?} lost the row");

            let explained = application
                .queries()
                .explain(
                    &connection_id,
                    &database,
                    "SELECT 1 AS astesia_milestone_4".to_string(),
                )
                .await
                .unwrap_or_else(|error| panic!("{engine:?} explain failed: {error}"));
            assert!(
                explained.success,
                "{engine:?} explain was unsuccessful: {:?}",
                explained.error
            );
        }

        let disconnected = application
            .connections()
            .perform_profile_operation(ProfileOperationCommand::Disconnect { connection_id })
            .await;
        assert!(matches!(
            disconnected.outcome,
            ProfileOperationOutcome::Disconnected(report) if report.is_complete()
        ));
    }
}

#[tokio::test]
#[ignore = "requires ASTESIA_ENGINE_SMOKE_CONFIG_PATH and seven disposable database engines"]
async fn milestone_five_catalog_object_and_grid_workflows() {
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
        let connection_id = smoke.config.id.clone();
        let connection_name = smoke.config.name.clone();
        application
            .connections()
            .save_profile(ValidatedProfile::from_request(SaveConnectionRequest {
                config: smoke.config.clone(),
                expected_revision: None,
                mcp_enabled: false,
                group_name: Some("Native milestone 5 smoke".to_string()),
                tags: vec!["native-m5-smoke".to_string()],
            }))
            .await
            .unwrap_or_else(|error| panic!("{engine:?} configure failed: {error}"));
        let connected = application
            .connections()
            .perform_profile_operation(ProfileOperationCommand::Connect {
                connection_id: connection_id.clone(),
            })
            .await;
        assert!(matches!(
            connected.outcome,
            ProfileOperationOutcome::Connected(Ok(ConnectionOutcome::Succeeded))
        ));

        let loaded = application
            .connections()
            .load_databases(&connection_id)
            .await
            .unwrap_or_else(|error| panic!("{engine:?} database load failed: {error}"));
        let database = smoke
            .browse_database
            .or_else(|| smoke.config.database.clone())
            .or_else(|| loaded.databases.first().cloned())
            .unwrap_or_else(|| panic!("{engine:?} returned no browseable database"));
        let target = QueryTarget {
            connection_id: connection_id.clone(),
            connection_name,
            database: database.clone(),
            db_type: engine,
            session_generation: loaded.session_generation,
        };
        let catalog = load_catalog(&application, &target).await;
        assert_catalog_contract(engine, &catalog);

        if engine.capabilities().sql {
            smoke_sql5_milestone_five(&application, &target, directory.path()).await;
        }

        let disconnected = application
            .connections()
            .perform_profile_operation(ProfileOperationCommand::Disconnect { connection_id })
            .await;
        assert!(matches!(
            disconnected.outcome,
            ProfileOperationOutcome::Disconnected(report) if report.is_complete()
        ));
    }
}

async fn load_catalog(
    application: &Application,
    target: &QueryTarget,
) -> super::DatabaseCatalogSnapshot {
    let mut catalog = super::DatabaseCatalogSnapshot::loading(target.db_type);
    for kind in catalog.pending_kinds() {
        let result = application
            .catalog()
            .catalog_section(&target.connection_id, &target.database, kind)
            .await;
        catalog.apply(result);
    }
    catalog
}

fn assert_catalog_contract(engine: DbType, catalog: &super::DatabaseCatalogSnapshot) {
    assert_section_ready(&catalog.tables, true, engine, "tables");
    let capabilities = engine.capabilities();
    assert_section_ready(&catalog.schemas, capabilities.schemas, engine, "schemas");
    assert_section_ready(&catalog.views, capabilities.views, engine, "views");
    assert_section_ready(
        &catalog.functions,
        capabilities.functions,
        engine,
        "functions",
    );
    assert_section_ready(
        &catalog.procedures,
        capabilities.procedures,
        engine,
        "procedures",
    );
    assert_section_ready(&catalog.triggers, capabilities.triggers, engine, "triggers");
    assert_section_ready(&catalog.users, capabilities.users, engine, "users");
}

fn assert_section_ready<T>(
    section: &CatalogSection<T>,
    supported: bool,
    engine: DbType,
    label: &str,
) {
    match (supported, section) {
        (true, CatalogSection::Ready(_)) | (false, CatalogSection::Unsupported) => {}
        (true, CatalogSection::Loading) => {
            panic!("{engine:?} {label} catalog was still loading")
        }
        (true, CatalogSection::Failed(error)) => {
            panic!("{engine:?} {label} catalog failed: {error}")
        }
        (true, CatalogSection::Unsupported) => {
            panic!("{engine:?} {label} catalog was unexpectedly unsupported")
        }
        (false, CatalogSection::Ready(_)) => {
            panic!("{engine:?} exposed unsupported {label} catalog data")
        }
        (false, CatalogSection::Loading) => {
            panic!("{engine:?} started unsupported {label} catalog work")
        }
        (false, CatalogSection::Failed(error)) => {
            panic!("{engine:?} queried unsupported {label}: {error}")
        }
    }
}

async fn smoke_sql5_milestone_five(
    application: &Application,
    target: &QueryTarget,
    output_directory: &std::path::Path,
) {
    let engine = target.db_type;
    let original_name = "astesia_m5_smoke";
    let table_name = "astesia_m5_smoke_archive";
    let view_name = "astesia_m5_smoke_view";
    let table = TableRef::unqualified(table_name);
    let drop_original = ObjectMutation::Drop(DropObjectTarget::Table(original_name.to_string()));
    let drop_table = ObjectMutation::Drop(DropObjectTarget::Table(table_name.to_string()));
    let drop_view = ObjectMutation::Drop(DropObjectTarget::View(view_name.to_string()));
    let _ = application.objects().execute(target, &drop_view).await;
    let _ = application.objects().execute(target, &drop_table).await;
    let _ = application.objects().execute(target, &drop_original).await;

    application
        .objects()
        .execute(
            target,
            &ObjectMutation::Create(CreateObjectSpec::Table {
                name: original_name.to_string(),
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
        .unwrap_or_else(|error| panic!("{engine:?} table creation failed: {error}"));
    application
        .objects()
        .execute(
            target,
            &ObjectMutation::Rename {
                kind: DatabaseObjectKind::Table,
                name: original_name.to_string(),
                new_name: table_name.to_string(),
            },
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} table rename failed: {error}"));

    let dialect = SqlDialect::new(engine);
    let quoted_table = dialect.quote_table_ref(&table).expect("quote smoke table");
    application
        .queries()
        .execute(
            &target.connection_id,
            &target.database,
            &format!(
                "INSERT INTO {quoted_table} ({}, {}) VALUES (1, 'Ada'), (2, 'Lin')",
                dialect.quote_identifier("id").unwrap(),
                dialect.quote_identifier("label").unwrap()
            ),
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} table seed failed: {error}"));

    let structure = application
        .catalog()
        .table_structure(&target.connection_id, &target.database, &table)
        .await
        .unwrap_or_else(|error| panic!("{engine:?} structure failed: {error:?}"));
    assert_eq!(structure.columns.len(), 2, "{engine:?} lost columns");
    assert_eq!(
        structure.constraints.is_some(),
        engine.capabilities().constraints,
        "{engine:?} constraint section did not follow capabilities"
    );
    assert_eq!(
        structure.foreign_keys.is_some(),
        engine.capabilities().foreign_keys,
        "{engine:?} foreign-key section did not follow capabilities"
    );

    let mut browsing = GridSession::new(target.clone(), table.clone(), 1).unwrap();
    load_grid(application, &mut browsing, engine).await;
    browsing
        .set_query_options(
            Some("label LIKE '%'".to_string()),
            vec![GridSort {
                column: "id".to_string(),
                direction: GridSortDirection::Descending,
            }],
        )
        .unwrap();
    load_grid(application, &mut browsing, engine).await;
    assert_eq!(grid_id(&browsing, 0), 2, "{engine:?} sort failed");
    browsing
        .select_cell(GridCell { row: 0, column: 0 }, false)
        .unwrap();
    browsing
        .select_cell(GridCell { row: 0, column: 1 }, true)
        .unwrap();
    assert_eq!(
        browsing.selection_tsv(true).as_deref(),
        Some("id\tlabel\n2\tLin"),
        "{engine:?} selection copy failed"
    );
    browsing.set_page(2).unwrap();
    load_grid(application, &mut browsing, engine).await;
    assert_eq!(grid_id(&browsing, 0), 1, "{engine:?} paging failed");

    let (columns, rows) = browsing.export_rows().expect("exportable smoke page");
    let export_path = output_directory.join(format!("{engine:?}-m5.csv"));
    let exported = application
        .exports()
        .export(
            &target.connection_id,
            &target.database,
            ExportSource::Rows { columns, rows },
            ExportFormat::Csv(CsvOptions {
                delimiter: ",".to_string(),
                include_header: true,
                quote_all: false,
                null_value: "\\N".to_string(),
                crlf: false,
                bom: false,
            }),
            export_path.display().to_string(),
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} export failed: {error}"));
    assert_eq!(exported, 1, "{engine:?} exported the wrong page");
    assert!(
        std::fs::read_to_string(&export_path)
            .expect("read smoke export")
            .contains("1,Ada"),
        "{engine:?} export lost selected rows"
    );

    let mut mutations = GridSession::new(target.clone(), table.clone(), 100).unwrap();
    load_grid(application, &mut mutations, engine).await;
    if engine == DbType::ClickHouse {
        assert_eq!(
            mutations.editability(),
            GridEditability::ReadOnlyEngine(DbType::ClickHouse)
        );
        assert_eq!(
            mutations.stage_insert(),
            Err(GridSessionError::ReadOnlyEngine(DbType::ClickHouse))
        );
    } else {
        assert!(matches!(
            mutations.editability(),
            GridEditability::Editable { .. }
        ));
        let row_one = grid_row_index(&mutations, 1);
        let row_two = grid_row_index(&mutations, 2);
        mutations
            .stage_cell_value(
                GridCell {
                    row: row_one,
                    column: 1,
                },
                json!("Augusta"),
            )
            .unwrap();
        mutations
            .select_row(row_two, GridRowSelectionMode::Replace)
            .unwrap();
        mutations.stage_delete_selection().unwrap();
        let draft = mutations.stage_insert().unwrap();
        mutations.set_draft_value(draft, 0, json!(3)).unwrap();
        mutations.set_draft_value(draft, 1, json!("Grace")).unwrap();
        let request = mutations.begin_save().unwrap();
        let outcome = application
            .grids()
            .save(&request)
            .await
            .unwrap_or_else(|error| panic!("{engine:?} grid save failed: {}", error.message));
        assert_eq!(outcome.changes_applied, 3, "{engine:?} lost a grid change");
        assert!(mutations.finish_save(&request, Ok(())));
        load_grid(application, &mut mutations, engine).await;
        let row_one = grid_row_index(&mutations, 1);
        assert_eq!(
            mutations.page().unwrap().rows[row_one][1],
            json!("Augusta"),
            "{engine:?} update did not round-trip"
        );
        assert!(
            mutations
                .page()
                .unwrap()
                .rows
                .iter()
                .all(|row| row[0] != json!(2)),
            "{engine:?} delete did not round-trip"
        );
        assert!(
            mutations
                .page()
                .unwrap()
                .rows
                .iter()
                .any(|row| row[0] == json!(3)),
            "{engine:?} insert did not round-trip"
        );

        let row_one = grid_row_index(&mutations, 1);
        mutations
            .stage_cell_value(
                GridCell {
                    row: row_one,
                    column: 1,
                },
                json!("Must Roll Back"),
            )
            .unwrap();
        let duplicate = mutations.stage_insert().unwrap();
        mutations.set_draft_value(duplicate, 0, json!(3)).unwrap();
        mutations
            .set_draft_value(duplicate, 1, json!("Duplicate"))
            .unwrap();
        let request = mutations.begin_save().unwrap();
        let failure = application
            .grids()
            .save(&request)
            .await
            .expect_err("duplicate primary key must fail the transaction");
        assert_eq!(
            failure.completed_statements, 1,
            "{engine:?} failed before exercising transactional rollback"
        );
        assert!(mutations.finish_save(&request, Err(failure)));
        let persisted = application
            .queries()
            .execute(
                &target.connection_id,
                &target.database,
                &format!(
                    "SELECT {} FROM {quoted_table} WHERE {} = 1",
                    dialect.quote_identifier("label").unwrap(),
                    dialect.quote_identifier("id").unwrap(),
                ),
            )
            .await
            .unwrap_or_else(|error| panic!("{engine:?} rollback verification failed: {error}"));
        assert_eq!(
            persisted.rows[0][0],
            json!("Augusta"),
            "{engine:?} kept a statement from a failed mutation batch"
        );
        assert!(mutations.discard_changes());
    }

    smoke_supported_object_kinds(application, target, &table).await;

    let create_view = ObjectMutation::Create(CreateObjectSpec::View {
        name: view_name.to_string(),
        query: format!("SELECT * FROM {quoted_table}"),
    });
    let view_sql = super::object_service::render_object_mutation(engine, &create_view)
        .expect("render smoke view");
    application
        .objects()
        .execute(target, &create_view)
        .await
        .unwrap_or_else(|error| {
            panic!("{engine:?} view creation failed: {error}; rendered SQL: {view_sql}")
        });
    let catalog = load_catalog(application, target).await;
    let CatalogSection::Ready(tables) = &catalog.tables else {
        panic!("{engine:?} tables were not ready after creation")
    };
    assert!(tables
        .iter()
        .any(|item| item.reference.name() == table_name));
    let CatalogSection::Ready(views) = &catalog.views else {
        panic!("{engine:?} views were not ready after creation")
    };
    let view = views
        .iter()
        .find(|view| view.name.ends_with(view_name))
        .unwrap_or_else(|| panic!("{engine:?} created view was not cataloged"));
    assert!(
        view.definition
            .as_ref()
            .is_some_and(|value| !value.is_empty()),
        "{engine:?} created view had no definition"
    );

    application
        .objects()
        .execute(target, &drop_view)
        .await
        .unwrap_or_else(|error| panic!("{engine:?} view drop failed: {error}"));
    application
        .objects()
        .execute(target, &drop_table)
        .await
        .unwrap_or_else(|error| panic!("{engine:?} table drop failed: {error}"));
    let catalog = load_catalog(application, target).await;
    let CatalogSection::Ready(tables) = catalog.tables else {
        panic!("{engine:?} tables were not ready after drop")
    };
    assert!(tables
        .iter()
        .all(|item| item.reference.name() != table_name));
}

async fn smoke_supported_object_kinds(
    application: &Application,
    target: &QueryTarget,
    table: &TableRef,
) {
    let engine = target.db_type;
    let capabilities = engine.capabilities();

    if capabilities.database_management {
        let name = "astesia_m5_database_smoke";
        let drop = ObjectMutation::Drop(DropObjectTarget::Database(name.to_string()));
        let _ = application.objects().execute(target, &drop).await;
        execute_smoke_object(
            application,
            target,
            ObjectMutation::Create(CreateObjectSpec::Database {
                name: name.to_string(),
            }),
        )
        .await;
        let databases = application
            .catalog()
            .databases(&target.connection_id)
            .await
            .unwrap_or_else(|error| panic!("{engine:?} database refresh failed: {error}"));
        assert!(databases.iter().any(|database| database == name));
        execute_smoke_object(application, target, drop).await;
    }

    if capabilities.schema_management {
        let name = "astesia_m5_schema_smoke";
        let drop = ObjectMutation::Drop(DropObjectTarget::Schema(name.to_string()));
        let _ = application.objects().execute(target, &drop).await;
        execute_smoke_object(
            application,
            target,
            ObjectMutation::Create(CreateObjectSpec::Schema {
                name: name.to_string(),
            }),
        )
        .await;
        if capabilities.schemas {
            let schemas = application
                .catalog()
                .schemas(&target.connection_id, &target.database)
                .await
                .unwrap_or_else(|error| panic!("{engine:?} schema refresh failed: {error}"));
            assert!(schemas.iter().any(|schema| schema == name));
        } else {
            let databases = application
                .catalog()
                .databases(&target.connection_id)
                .await
                .unwrap_or_else(|error| panic!("{engine:?} schema alias refresh failed: {error}"));
            assert!(databases.iter().any(|database| database == name));
        }
        execute_smoke_object(application, target, drop).await;
    }

    if capabilities.functions {
        let (create, drop_name) = smoke_function(engine);
        let drop = ObjectMutation::Drop(DropObjectTarget::Function(drop_name));
        let _ = application.objects().execute(target, &drop).await;
        execute_smoke_object(application, target, ObjectMutation::Create(create)).await;
        let functions = application
            .catalog()
            .functions(&target.connection_id, &target.database)
            .await
            .unwrap_or_else(|error| panic!("{engine:?} function refresh failed: {error}"));
        assert!(functions
            .iter()
            .any(|function| function.name.contains("astesia_m5_function")));
        execute_smoke_object(application, target, drop).await;
    }

    if capabilities.procedures {
        let (create, drop_name) = smoke_procedure(engine);
        let drop = ObjectMutation::Drop(DropObjectTarget::Procedure(drop_name));
        let _ = application.objects().execute(target, &drop).await;
        execute_smoke_object(application, target, ObjectMutation::Create(create)).await;
        let procedures = application
            .catalog()
            .procedures(&target.connection_id, &target.database)
            .await
            .unwrap_or_else(|error| panic!("{engine:?} procedure refresh failed: {error}"));
        assert!(procedures
            .iter()
            .any(|procedure| procedure.name.contains("astesia_m5_procedure")));
        execute_smoke_object(application, target, drop).await;
    }

    if capabilities.triggers {
        let trigger_function_drop = if engine == DbType::PostgreSQL {
            let drop = ObjectMutation::Drop(DropObjectTarget::Function(
                "public.astesia_m5_trigger_function()".to_string(),
            ));
            let _ = application.objects().execute(target, &drop).await;
            execute_smoke_object(
                application,
                target,
                ObjectMutation::Create(CreateObjectSpec::Function {
                    name: "public.astesia_m5_trigger_function".to_string(),
                    arguments: String::new(),
                    return_type: "trigger".to_string(),
                    language: "plpgsql".to_string(),
                    body: "BEGIN\nRETURN NEW;\nEND;".to_string(),
                }),
            )
            .await;
            Some(drop)
        } else {
            None
        };
        let name = match engine {
            DbType::PostgreSQL => "public.astesia_m5_trigger",
            DbType::SQLServer => "dbo.astesia_m5_trigger",
            _ => "astesia_m5_trigger",
        };
        let drop = ObjectMutation::Drop(DropObjectTarget::Trigger {
            name: name.to_string(),
            table: table.to_string(),
        });
        let _ = application.objects().execute(target, &drop).await;
        execute_smoke_object(
            application,
            target,
            ObjectMutation::Create(CreateObjectSpec::Trigger {
                name: name.to_string(),
                table: table.clone(),
                timing: TriggerTiming::After,
                event: TriggerEvent::Insert,
                body: match engine {
                    DbType::PostgreSQL => "public.astesia_m5_trigger_function()".to_string(),
                    DbType::MySQL => "SET @astesia_m5_trigger_seen = NEW.id;".to_string(),
                    DbType::SQLite => "SELECT NEW.id;".to_string(),
                    DbType::SQLServer => "SET NOCOUNT ON;".to_string(),
                    _ => unreachable!("trigger capability is disabled"),
                },
            }),
        )
        .await;
        let triggers = application
            .catalog()
            .triggers(&target.connection_id, &target.database)
            .await
            .unwrap_or_else(|error| panic!("{engine:?} trigger refresh failed: {error}"));
        let trigger = triggers
            .iter()
            .find(|trigger| trigger.name.contains("astesia_m5_trigger"))
            .unwrap_or_else(|| panic!("{engine:?} created trigger was not cataloged"));
        assert_eq!(trigger.timing, "AFTER");
        execute_smoke_object(application, target, drop).await;
        if let Some(drop) = trigger_function_drop {
            execute_smoke_object(application, target, drop).await;
        }
    }

    if capabilities.users {
        let name = "astesia_m5_user";
        let host = (engine == DbType::MySQL).then(|| "%".to_string());
        let drop = ObjectMutation::Drop(DropObjectTarget::User {
            name: name.to_string(),
            host: host.clone(),
        });
        let _ = application.objects().execute(target, &drop).await;
        execute_smoke_object(
            application,
            target,
            ObjectMutation::Create(CreateObjectSpec::User {
                name: name.to_string(),
                host,
                password: "Astesia-M5!2026".to_string(),
            }),
        )
        .await;
        let users = application
            .catalog()
            .users(&target.connection_id)
            .await
            .unwrap_or_else(|error| panic!("{engine:?} user refresh failed: {error}"));
        assert!(users.iter().any(|user| user.name == name));
        execute_smoke_object(application, target, drop).await;
    }
}

async fn execute_smoke_object(
    application: &Application,
    target: &QueryTarget,
    mutation: ObjectMutation,
) {
    let engine = target.db_type;
    let kind = mutation.kind();
    let identity = mutation.display_identity();
    application
        .objects()
        .execute(target, &mutation)
        .await
        .unwrap_or_else(|error| panic!("{engine:?} {kind:?} {identity} failed: {error}"));
}

fn smoke_function(engine: DbType) -> (CreateObjectSpec, String) {
    let (name, arguments, return_type, language, body, drop_name) = match engine {
        DbType::PostgreSQL => (
            "public.astesia_m5_function",
            "value_in integer",
            "integer",
            "sql",
            "SELECT value_in + 1",
            "public.astesia_m5_function(integer)",
        ),
        DbType::MySQL => (
            "astesia_m5_function",
            "value_in INT",
            "INT",
            "SQL",
            "RETURN value_in + 1;",
            "astesia_m5_function",
        ),
        DbType::SQLServer => (
            "dbo.astesia_m5_function",
            "@value_in INT",
            "INT",
            "T-SQL",
            "RETURN @value_in + 1;",
            "dbo.astesia_m5_function",
        ),
        DbType::ClickHouse => (
            "astesia_m5_function",
            "",
            "",
            "SQL",
            "(value_in) -> value_in + 1",
            "astesia_m5_function",
        ),
        _ => unreachable!("function capability is disabled"),
    };
    (
        CreateObjectSpec::Function {
            name: name.to_string(),
            arguments: arguments.to_string(),
            return_type: return_type.to_string(),
            language: language.to_string(),
            body: body.to_string(),
        },
        drop_name.to_string(),
    )
}

fn smoke_procedure(engine: DbType) -> (CreateObjectSpec, String) {
    let (name, arguments, language, body, drop_name) = match engine {
        DbType::PostgreSQL => (
            "public.astesia_m5_procedure",
            "value_in integer",
            "plpgsql",
            "BEGIN\nPERFORM value_in;\nEND;",
            "public.astesia_m5_procedure(integer)",
        ),
        DbType::MySQL => (
            "astesia_m5_procedure",
            "IN value_in INT",
            "SQL",
            "SELECT value_in;",
            "astesia_m5_procedure",
        ),
        DbType::SQLServer => (
            "dbo.astesia_m5_procedure",
            "@value_in INT",
            "T-SQL",
            "SELECT @value_in AS value_in;",
            "dbo.astesia_m5_procedure",
        ),
        _ => unreachable!("procedure capability is disabled"),
    };
    (
        CreateObjectSpec::Procedure {
            name: name.to_string(),
            arguments: arguments.to_string(),
            language: language.to_string(),
            body: body.to_string(),
        },
        drop_name.to_string(),
    )
}

async fn load_grid(application: &Application, session: &mut GridSession, engine: DbType) {
    let request = session.begin_load().unwrap();
    let page = application
        .grids()
        .load(&request)
        .await
        .unwrap_or_else(|error| panic!("{engine:?} grid load failed: {error}"));
    assert!(session.finish_load(&request, Ok(page)));
}

fn grid_id(session: &GridSession, row: usize) -> i64 {
    session.page().unwrap().rows[row][0]
        .as_i64()
        .or_else(|| {
            session.page().unwrap().rows[row][0]
                .as_u64()
                .and_then(|value| i64::try_from(value).ok())
        })
        .expect("numeric smoke id")
}

fn grid_row_index(session: &GridSession, id: i64) -> usize {
    (0..session.page().unwrap().rows.len())
        .find(|row| grid_id(session, *row) == id)
        .unwrap_or_else(|| panic!("smoke row {id} was not found"))
}

fn smoke_integer_type(engine: DbType) -> &'static str {
    match engine {
        DbType::ClickHouse => "Int64",
        _ => "INTEGER",
    }
}

fn smoke_text_type(engine: DbType) -> &'static str {
    match engine {
        DbType::MySQL => "VARCHAR(64)",
        DbType::SQLServer => "NVARCHAR(64)",
        DbType::ClickHouse => "String",
        _ => "TEXT",
    }
}
