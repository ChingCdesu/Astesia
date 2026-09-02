use std::collections::HashSet;
use std::path::PathBuf;

use serde::Deserialize;

use super::{
    Application, ConnectionOutcome, ProfileOperationCommand, ProfileOperationOutcome,
    ValidatedProfile,
};
use crate::connection_repository::{SaveConnectionRequest, SharedConnectionRepository};
use crate::credential_vault::test_support::MemoryCredentialVault;
use crate::db::{ConnectionConfig, DbType};

#[derive(Deserialize)]
struct SmokeTarget {
    config: ConnectionConfig,
    browse_database: Option<String>,
}

#[tokio::test]
#[ignore = "requires ASTESIA_ENGINE_SMOKE_CONFIG_PATH and seven disposable database engines"]
async fn all_engines_cross_the_application_connection_workflow() {
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

    let directory = tempfile::tempdir().expect("tempdir");
    let repository = SharedConnectionRepository::new(
        directory.path().join("connections.sqlite3"),
        MemoryCredentialVault::shared(),
    );
    let application = Application::with_repository(repository);

    for target in targets {
        let engine = target.config.db_type;
        if target_engine.is_some() && target_engine != Some(engine) {
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
