use super::{
    milestone_six::connect_smoke_target, smoke_inputs, smoke_integer_type, smoke_text_type,
};
use crate::application::{
    Application, ChartModel, ChartType, CreateObjectSpec, GridQuery, ObjectMutation,
    PerformanceSnapshot, ProfileOperationCommand, ProfileOperationOutcome, TableColumnSpec,
};
use crate::connection_repository::SharedConnectionRepository;
use crate::credential_vault::test_support::MemoryCredentialVault;
use crate::db::{DbType, SqlDialect, TableRef};

#[tokio::test]
#[ignore = "requires ASTESIA_ENGINE_SMOKE_CONFIG_PATH and seven disposable database engines"]
async fn milestone_seven_visualization_and_diagnostics_workflows() {
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
        let target = connect_smoke_target(&application, &smoke, "Native milestone 7 smoke").await;

        let metrics = application
            .performance()
            .metrics(&target)
            .await
            .unwrap_or_else(|error| panic!("{engine:?} performance metrics failed: {error}"));
        assert_performance_engine(engine, metrics);

        if engine.capabilities().sql {
            smoke_charts(&application, &target).await;
        }
        if engine.capabilities().foreign_keys {
            smoke_er_diagram(&application, &target).await;
        }

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

fn assert_performance_engine(engine: DbType, snapshot: PerformanceSnapshot) {
    assert!(
        matches!(
            (engine, snapshot),
            (DbType::MySQL, PerformanceSnapshot::MySql(_))
                | (DbType::PostgreSQL, PerformanceSnapshot::PostgreSql(_))
                | (DbType::SQLite, PerformanceSnapshot::SQLite(_))
                | (DbType::SQLServer, PerformanceSnapshot::SqlServer(_))
                | (DbType::MongoDB, PerformanceSnapshot::MongoDB(_))
                | (DbType::Redis, PerformanceSnapshot::Redis(_))
                | (DbType::ClickHouse, PerformanceSnapshot::ClickHouse(_))
        ),
        "{engine:?} returned the wrong performance snapshot"
    );
}

async fn smoke_charts(application: &Application, target: &crate::application::QueryTarget) {
    let engine = target.db_type;
    let result = application
        .queries()
        .execute(
            &target.connection_id,
            &target.database,
            "SELECT 1 AS ordinal, 'East' AS region, 3 AS revenue UNION ALL SELECT 2, 'West', 5 UNION ALL SELECT 3, 'East', 7",
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} chart query failed: {error}"));
    let mut model = ChartModel::new(&result.columns, &result.rows);
    for chart_type in [
        ChartType::Bar,
        ChartType::Line,
        ChartType::Area,
        ChartType::Pie,
    ] {
        model.set_chart_type(chart_type);
        let series = model
            .series()
            .unwrap_or_else(|error| panic!("{engine:?} {chart_type:?} mapping failed: {error:?}"));
        assert!(series.iter().any(|series| !series.points.is_empty()));
    }
    model.set_x_column(0);
    model.set_chart_type(ChartType::Scatter);
    assert!(!model.series().expect("numeric scatter mapping").is_empty());

    let table = TableRef::unqualified("astesia_m7_chart");
    let dialect = SqlDialect::new(engine);
    let quoted = dialect.quote_table_ref(&table).expect("quote chart table");
    let _ = application
        .queries()
        .execute(
            &target.connection_id,
            &target.database,
            &format!("DROP TABLE IF EXISTS {quoted}"),
        )
        .await;
    application
        .objects()
        .execute(
            target,
            &ObjectMutation::Create(CreateObjectSpec::Table {
                name: table.name().to_string(),
                columns: vec![
                    TableColumnSpec {
                        name: "id".to_string(),
                        data_type: smoke_integer_type(engine).to_string(),
                        nullable: false,
                        primary_key: true,
                        default_value: None,
                    },
                    TableColumnSpec {
                        name: "region".to_string(),
                        data_type: smoke_text_type(engine).to_string(),
                        nullable: false,
                        primary_key: false,
                        default_value: None,
                    },
                    TableColumnSpec {
                        name: "revenue".to_string(),
                        data_type: smoke_integer_type(engine).to_string(),
                        nullable: false,
                        primary_key: false,
                        default_value: None,
                    },
                ],
            }),
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} chart table creation failed: {error}"));
    application
        .queries()
        .execute(
            &target.connection_id,
            &target.database,
            &format!("INSERT INTO {quoted} VALUES (1, 'East', 3), (2, 'West', 5), (3, 'East', 7)"),
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} chart table seed failed: {error}"));
    let table_data = application
        .charts()
        .table_data(
            target.clone(),
            table.clone(),
            GridQuery {
                page: 1,
                page_size: 100,
                filter: None,
                sort: vec![],
            },
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} chart table load failed: {error}"));
    assert_eq!(table_data.rows.len(), 3);
    assert!(table_data.columns.iter().any(|column| column == "revenue"));
    let _ = application
        .queries()
        .execute(
            &target.connection_id,
            &target.database,
            &format!("DROP TABLE IF EXISTS {quoted}"),
        )
        .await;
}

async fn smoke_er_diagram(application: &Application, target: &crate::application::QueryTarget) {
    let engine = target.db_type;
    let dialect = SqlDialect::new(engine);
    let parent = TableRef::unqualified("astesia_m7_parent");
    let child = TableRef::unqualified("astesia_m7_child");
    let quoted_parent = dialect.quote_table_ref(&parent).expect("quote parent");
    let quoted_child = dialect.quote_table_ref(&child).expect("quote child");
    for table in [&quoted_child, &quoted_parent] {
        let _ = application
            .queries()
            .execute(
                &target.connection_id,
                &target.database,
                &format!("DROP TABLE IF EXISTS {table}"),
            )
            .await;
    }
    application
        .queries()
        .execute(
            &target.connection_id,
            &target.database,
            &format!("CREATE TABLE {quoted_parent} (id INTEGER PRIMARY KEY)"),
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} ER parent creation failed: {error}"));
    application
        .queries()
        .execute(
            &target.connection_id,
            &target.database,
            &format!(
                "CREATE TABLE {quoted_child} (id INTEGER PRIMARY KEY, parent_id INTEGER NOT NULL, CONSTRAINT astesia_m7_fk FOREIGN KEY (parent_id) REFERENCES {quoted_parent} (id))"
            ),
        )
        .await
        .unwrap_or_else(|error| panic!("{engine:?} ER child creation failed: {error}"));

    let schema = application
        .er_diagrams()
        .load(target)
        .await
        .unwrap_or_else(|error| panic!("{engine:?} ER diagram load failed: {error}"));
    assert!(schema
        .tables
        .iter()
        .any(|table| table.reference.name() == parent.name()));
    assert!(schema.relationships.iter().any(|relationship| {
        relationship.from_table.name() == child.name()
            && relationship.to_table.name() == parent.name()
            && relationship.from_columns == ["parent_id"]
            && relationship.to_columns == ["id"]
    }));

    for table in [&quoted_child, &quoted_parent] {
        application
            .queries()
            .execute(
                &target.connection_id,
                &target.database,
                &format!("DROP TABLE IF EXISTS {table}"),
            )
            .await
            .unwrap_or_else(|error| panic!("{engine:?} ER cleanup failed: {error}"));
    }
}
