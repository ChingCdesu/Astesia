use serde_json::json;

use super::*;

use crate::application::{ConnectionProfileSnapshot, DatabaseSessionSnapshot};
use crate::connection_repository::{ConnectionRepositoryErrorCode, SharedConnectionProfile};
use crate::db::DbType;

fn profile(id: &str, db_type: DbType) -> SharedConnectionProfile {
    SharedConnectionProfile {
        id: id.to_string(),
        name: id.to_string(),
        db_type,
        host: if id == "sqlite" {
            "/tmp/astesia.sqlite3".to_string()
        } else {
            "127.0.0.1".to_string()
        },
        port: 5432,
        username: "tester".to_string(),
        database: None,
        color: None,
        group_name: None,
        tags: Vec::new(),
        has_credential: false,
        revision: 1,
        mcp_enabled: false,
    }
}

fn snapshot(revision: i64, profiles: Vec<SharedConnectionProfile>) -> ConnectionWorkspaceSnapshot {
    ConnectionWorkspaceSnapshot {
        repository_revision: revision,
        mcp_revision: 0,
        profiles: profiles
            .into_iter()
            .map(|profile| ConnectionProfileSnapshot {
                profile,
                session: DatabaseSessionSnapshot { generation: None },
                mcp_usage: None,
            })
            .collect(),
    }
}

fn connected_snapshot(
    revision: i64,
    profile: SharedConnectionProfile,
    session_generation: u64,
) -> ConnectionWorkspaceSnapshot {
    ConnectionWorkspaceSnapshot {
        repository_revision: revision,
        mcp_revision: 0,
        profiles: vec![ConnectionProfileSnapshot {
            profile,
            session: DatabaseSessionSnapshot {
                generation: Some(session_generation),
            },
            mcp_usage: None,
        }],
    }
}

fn error(message: &str) -> ConnectionWorkspaceError {
    ConnectionWorkspaceError::from(ConnectionRepositoryError {
        code: ConnectionRepositoryErrorCode::StorageUnavailable,
        message: message.to_string(),
        remediation: "Retry".to_string(),
        retryable: true,
        details: Box::new(json!({})),
    })
}

#[test]
fn initial_refresh_transitions_to_loaded_or_error() {
    let mut state = ConnectionWorkspaceState::default();
    let request = state.begin_refresh();
    assert!(state.is_refreshing());
    assert_eq!(
        state.finish_refresh(request, Ok(snapshot(4, Vec::new()))),
        SnapshotApply::Applied
    );
    assert!(!state.is_refreshing());
    assert_eq!(
        state
            .snapshot()
            .map(|snapshot| snapshot.repository_revision),
        Some(4)
    );

    let request = state.begin_refresh();
    assert_eq!(
        state.finish_refresh(request, Err(error("unavailable"))),
        SnapshotApply::Failed
    );
    assert_eq!(
        state
            .snapshot()
            .map(|snapshot| snapshot.repository_revision),
        Some(4)
    );
    assert_eq!(
        state.error().map(|error| error.message.as_str()),
        Some("unavailable")
    );
}

#[test]
fn stale_refresh_cannot_replace_a_newer_snapshot() {
    let mut state = ConnectionWorkspaceState::default();
    let stale = state.begin_refresh();
    let current = state.begin_refresh();

    assert_eq!(
        state.finish_refresh(current, Ok(snapshot(2, Vec::new()))),
        SnapshotApply::Applied
    );
    assert_eq!(
        state.finish_refresh(stale, Ok(snapshot(1, Vec::new()))),
        SnapshotApply::Superseded
    );
    assert_eq!(
        state
            .snapshot()
            .map(|snapshot| snapshot.repository_revision),
        Some(2)
    );
}

#[test]
fn operation_snapshot_supersedes_an_older_refresh() {
    let mut state = ConnectionWorkspaceState::default();
    let refresh = state.begin_refresh();
    let operation = state
        .begin_operation("primary", ProfileOperationKind::Connecting)
        .expect("operation starts");

    assert_eq!(
        state.finish_refresh(refresh, Ok(snapshot(1, Vec::new()))),
        SnapshotApply::Superseded
    );
    assert_eq!(
        state.finish_operation(&operation, Ok(snapshot(2, Vec::new()))),
        OperationApply::Snapshot(SnapshotApply::Applied)
    );
    assert_eq!(
        state
            .snapshot()
            .map(|snapshot| snapshot.repository_revision),
        Some(2)
    );
}

#[test]
fn operations_finishing_out_of_order_keep_the_latest_snapshot() {
    let mut state = ConnectionWorkspaceState::default();
    let first = state
        .begin_operation("first", ProfileOperationKind::Connecting)
        .expect("first operation starts");
    let second = state
        .begin_operation("second", ProfileOperationKind::Disconnecting)
        .expect("second operation starts");

    assert_eq!(
        state.finish_operation(
            &second,
            Ok(snapshot(
                8,
                vec![
                    profile("first", DbType::PostgreSQL),
                    profile("second", DbType::SQLite),
                ],
            )),
        ),
        OperationApply::Snapshot(SnapshotApply::Applied)
    );
    assert_eq!(
        state.finish_operation(&first, Ok(snapshot(7, Vec::new()))),
        OperationApply::Snapshot(SnapshotApply::Superseded)
    );
    assert_eq!(
        state
            .snapshot()
            .map(|snapshot| snapshot.repository_revision),
        Some(8)
    );
}

#[test]
fn snapshot_failure_preserves_rows_and_marks_them_stale() {
    let mut state = ConnectionWorkspaceState::default();
    let refresh = state.begin_refresh();
    state.finish_refresh(
        refresh,
        Ok(snapshot(3, vec![profile("primary", DbType::PostgreSQL)])),
    );
    let operation = state
        .begin_operation("primary", ProfileOperationKind::Connecting)
        .expect("operation starts");

    assert_eq!(
        state.finish_operation(&operation, Err(error("refresh failed"))),
        OperationApply::Snapshot(SnapshotApply::Failed)
    );
    assert_eq!(state.snapshot().unwrap().repository_revision, 3);
    assert_eq!(
        state.error().map(|error| error.message.as_str()),
        Some("refresh failed")
    );
}

#[test]
fn stale_database_result_cannot_cross_a_reconnected_session() {
    let mut state = ConnectionWorkspaceState::default();
    let profile = profile("primary", DbType::PostgreSQL);
    let refresh = state.begin_refresh();
    state.finish_refresh(refresh, Ok(connected_snapshot(1, profile.clone(), 4)));
    let request = state
        .begin_database_load("primary")
        .expect("database load starts");

    let reconnect = state
        .begin_operation("primary", ProfileOperationKind::Connecting)
        .expect("reconnect starts");
    state.finish_operation(&reconnect, Ok(connected_snapshot(1, profile, 5)));

    assert!(!state.finish_database_load(
        &request,
        Ok(LoadedDatabases {
            session_generation: 4,
            databases: vec!["stale".to_string()],
        }),
    ));
    assert!(state.databases("primary").is_none());
}

#[test]
fn connected_profile_loads_databases_once_per_session_generation() {
    let mut state = ConnectionWorkspaceState::default();
    let profile = profile("primary", DbType::PostgreSQL);
    let refresh = state.begin_refresh();
    state.finish_refresh(refresh, Ok(connected_snapshot(1, profile.clone(), 4)));

    assert!(state.begin_database_load("primary").is_some());
    assert!(state.begin_database_load("primary").is_none());

    let reconnect = state
        .begin_operation("primary", ProfileOperationKind::Connecting)
        .expect("reconnect starts");
    state.finish_operation(&reconnect, Ok(connected_snapshot(2, profile, 5)));
    assert!(state.begin_database_load("primary").is_some());
}

#[test]
fn query_targets_require_the_same_live_session_and_loaded_database() {
    let mut state = ConnectionWorkspaceState::default();
    let refresh = state.begin_refresh();
    state.finish_refresh(
        refresh,
        Ok(connected_snapshot(
            1,
            profile("primary", DbType::PostgreSQL),
            7,
        )),
    );
    let request = state.begin_database_load("primary").unwrap();
    state.finish_database_load(
        &request,
        Ok(LoadedDatabases {
            session_generation: 7,
            databases: vec!["app".to_string()],
        }),
    );

    let mut target = QueryTarget {
        connection_id: "primary".to_string(),
        connection_name: "Primary".to_string(),
        database: "app".to_string(),
        db_type: DbType::PostgreSQL,
        session_generation: 7,
    };
    assert!(state.query_target_is_live(&target));

    target.session_generation = 8;
    assert!(!state.query_target_is_live(&target));
    target.session_generation = 7;
    target.database = "missing".to_string();
    assert!(!state.query_target_is_live(&target));
}

#[test]
fn object_loading_is_lazy_and_rejects_stale_session_results() {
    let mut state = ConnectionWorkspaceState::default();
    let profile = profile("primary", DbType::PostgreSQL);
    let refresh = state.begin_refresh();
    state.finish_refresh(refresh, Ok(connected_snapshot(1, profile.clone(), 7)));
    let database_request = state.begin_database_load("primary").unwrap();
    state.finish_database_load(
        &database_request,
        Ok(LoadedDatabases {
            session_generation: 7,
            databases: vec!["app".to_string()],
        }),
    );
    let target = QueryTarget {
        connection_id: "primary".to_string(),
        connection_name: "Primary".to_string(),
        database: "app".to_string(),
        db_type: DbType::PostgreSQL,
        session_generation: 7,
    };
    let request = state
        .begin_object_load(&target)
        .expect("object load starts")
        .into_iter()
        .find(|request| request.kind() == CatalogKind::Tables)
        .expect("table load starts");
    assert!(state.begin_object_load(&target).is_none());

    let reconnect = state
        .begin_operation("primary", ProfileOperationKind::Connecting)
        .expect("reconnect starts");
    state.finish_operation(&reconnect, Ok(connected_snapshot(2, profile, 8)));

    assert!(!state.finish_object_load(&request, CatalogLoadResult::Tables(Ok(Vec::new()))));
    assert!(state.objects(&target).is_none());
}

#[test]
fn loaded_objects_are_scoped_to_database_and_session() {
    let mut state = ConnectionWorkspaceState::default();
    let refresh = state.begin_refresh();
    state.finish_refresh(
        refresh,
        Ok(connected_snapshot(1, profile("primary", DbType::SQLite), 3)),
    );
    let database_request = state.begin_database_load("primary").unwrap();
    state.finish_database_load(
        &database_request,
        Ok(LoadedDatabases {
            session_generation: 3,
            databases: vec!["main".to_string()],
        }),
    );
    let target = QueryTarget {
        connection_id: "primary".to_string(),
        connection_name: "Primary".to_string(),
        database: "main".to_string(),
        db_type: DbType::SQLite,
        session_generation: 3,
    };
    let request = state
        .begin_object_load(&target)
        .unwrap()
        .into_iter()
        .find(|request| request.kind() == CatalogKind::Tables)
        .unwrap();
    let table = TableInfo {
        reference: crate::db::TableRef::unqualified("users"),
        row_count: Some(4),
        comment: None,
    };

    assert!(state.finish_object_load(&request, CatalogLoadResult::Tables(Ok(vec![table]))));
    assert!(matches!(
        state.objects(&target),
        Some(ObjectListState::Ready {
            catalog: DatabaseCatalogSnapshot {
                tables: CatalogSection::Ready(tables),
                ..
            },
            ..
        }) if tables.len() == 1
    ));

    state.clear_object_state(&target);
    assert!(state.begin_object_load(&target).is_some());
    assert!(matches!(
        state.objects(&target),
        Some(ObjectListState::Ready {
            catalog: DatabaseCatalogSnapshot {
                tables: CatalogSection::Loading,
                ..
            },
            ..
        })
    ));
}
