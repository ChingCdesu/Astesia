use std::collections::HashMap;

use super::{ConnectionWorkspaceSnapshot, LoadedDatabases};
use crate::connection_repository::ConnectionRepositoryError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectionWorkspaceError {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) remediation: String,
}

impl ConnectionWorkspaceError {
    pub(crate) fn load_profiles(error: impl std::fmt::Display) -> Self {
        Self {
            code: "profile_load_task_failed".to_string(),
            message: format!("加载连接配置的后台任务意外结束：{error}"),
            remediation: "请重试；如果问题持续存在，请查看应用日志。".to_string(),
        }
    }

    pub(crate) fn operation(operation: &str, error: impl std::fmt::Display) -> Self {
        Self {
            code: "profile_operation_task_failed".to_string(),
            message: format!("{operation}后台任务意外结束：{error}"),
            remediation: "请刷新并确认当前状态；如果问题持续存在，请查看应用日志。".to_string(),
        }
    }

    pub(crate) fn startup(error: impl std::fmt::Display) -> Self {
        Self {
            code: "application_startup_task_failed".to_string(),
            message: format!("加载应用的后台任务意外结束：{error}"),
            remediation: "请重试；如果问题持续存在，请查看应用日志。".to_string(),
        }
    }
}

impl From<ConnectionRepositoryError> for ConnectionWorkspaceError {
    fn from(error: ConnectionRepositoryError) -> Self {
        Self {
            code: error.code.as_str().to_string(),
            message: error.message,
            remediation: error.remediation,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProfileOperationKind {
    Connecting,
    Disconnecting,
    Deleting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProfileOperation {
    generation: u64,
    kind: ProfileOperationKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RefreshRequest {
    epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProfileOperationRequest {
    connection_id: String,
    generation: u64,
    snapshot_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DatabaseLoadRequest {
    connection_id: String,
    session_generation: u64,
    request_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SnapshotApply {
    Applied,
    Failed,
    Superseded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OperationApply {
    Discarded,
    Snapshot(SnapshotApply),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DatabaseListState {
    Loading {
        session_generation: u64,
        request_generation: u64,
    },
    Ready {
        session_generation: u64,
        databases: Vec<String>,
    },
    Failed {
        session_generation: u64,
        error: String,
    },
}

impl DatabaseListState {
    fn session_generation(&self) -> u64 {
        match self {
            Self::Loading {
                session_generation, ..
            }
            | Self::Ready {
                session_generation, ..
            }
            | Self::Failed {
                session_generation, ..
            } => *session_generation,
        }
    }
}

#[derive(Debug)]
enum SnapshotState {
    Loading,
    Failed(ConnectionWorkspaceError),
    Ready {
        snapshot: ConnectionWorkspaceSnapshot,
        refresh: RefreshState,
    },
}

#[derive(Debug)]
enum RefreshState {
    Idle,
    Refreshing,
    Stale(ConnectionWorkspaceError),
}

pub(crate) struct ConnectionWorkspaceState {
    snapshot: SnapshotState,
    snapshot_epoch: u64,
    operation_generation: u64,
    database_request_generation: u64,
    operations: HashMap<String, ProfileOperation>,
    databases: HashMap<String, DatabaseListState>,
}

impl Default for ConnectionWorkspaceState {
    fn default() -> Self {
        Self {
            snapshot: SnapshotState::Loading,
            snapshot_epoch: 0,
            operation_generation: 0,
            database_request_generation: 0,
            operations: HashMap::new(),
            databases: HashMap::new(),
        }
    }
}

impl ConnectionWorkspaceState {
    pub(crate) fn snapshot(&self) -> Option<&ConnectionWorkspaceSnapshot> {
        match &self.snapshot {
            SnapshotState::Ready { snapshot, .. } => Some(snapshot),
            SnapshotState::Loading | SnapshotState::Failed(_) => None,
        }
    }

    pub(crate) fn error(&self) -> Option<&ConnectionWorkspaceError> {
        match &self.snapshot {
            SnapshotState::Failed(error)
            | SnapshotState::Ready {
                refresh: RefreshState::Stale(error),
                ..
            } => Some(error),
            SnapshotState::Loading
            | SnapshotState::Ready {
                refresh: RefreshState::Idle | RefreshState::Refreshing,
                ..
            } => None,
        }
    }

    pub(crate) fn is_refreshing(&self) -> bool {
        matches!(
            self.snapshot,
            SnapshotState::Loading
                | SnapshotState::Ready {
                    refresh: RefreshState::Refreshing,
                    ..
                }
        )
    }

    pub(crate) fn operation(&self, connection_id: &str) -> Option<ProfileOperationKind> {
        self.operations
            .get(connection_id)
            .map(|operation| operation.kind)
    }

    pub(crate) fn databases(&self, connection_id: &str) -> Option<&DatabaseListState> {
        self.databases.get(connection_id)
    }

    pub(crate) fn begin_refresh(&mut self) -> RefreshRequest {
        let request = RefreshRequest {
            epoch: self.next_snapshot_epoch(),
        };
        self.snapshot = match std::mem::replace(&mut self.snapshot, SnapshotState::Loading) {
            SnapshotState::Ready { snapshot, .. } => SnapshotState::Ready {
                snapshot,
                refresh: RefreshState::Refreshing,
            },
            SnapshotState::Loading | SnapshotState::Failed(_) => SnapshotState::Loading,
        };
        request
    }

    pub(crate) fn finish_refresh(
        &mut self,
        request: RefreshRequest,
        result: Result<ConnectionWorkspaceSnapshot, ConnectionWorkspaceError>,
    ) -> SnapshotApply {
        self.apply_snapshot(request.epoch, result)
    }

    pub(crate) fn begin_operation(
        &mut self,
        connection_id: &str,
        kind: ProfileOperationKind,
    ) -> Option<ProfileOperationRequest> {
        if self.operations.contains_key(connection_id) {
            return None;
        }

        self.operation_generation = self.operation_generation.wrapping_add(1);
        let generation = self.operation_generation;
        self.operations.insert(
            connection_id.to_string(),
            ProfileOperation { generation, kind },
        );
        Some(ProfileOperationRequest {
            connection_id: connection_id.to_string(),
            generation,
            snapshot_epoch: self.next_snapshot_epoch(),
        })
    }

    pub(crate) fn finish_operation(
        &mut self,
        request: &ProfileOperationRequest,
        snapshot: Result<ConnectionWorkspaceSnapshot, ConnectionWorkspaceError>,
    ) -> OperationApply {
        if self
            .operations
            .get(&request.connection_id)
            .is_none_or(|operation| operation.generation != request.generation)
        {
            return OperationApply::Discarded;
        }

        self.operations.remove(&request.connection_id);
        OperationApply::Snapshot(self.apply_snapshot(request.snapshot_epoch, snapshot))
    }

    pub(crate) fn begin_database_load(
        &mut self,
        connection_id: &str,
    ) -> Option<DatabaseLoadRequest> {
        let session_generation = self.session_generation(connection_id)?;
        if self
            .databases
            .get(connection_id)
            .is_some_and(|state| state.session_generation() == session_generation)
        {
            return None;
        }

        self.database_request_generation = self.database_request_generation.wrapping_add(1);
        let request_generation = self.database_request_generation;
        self.databases.insert(
            connection_id.to_string(),
            DatabaseListState::Loading {
                session_generation,
                request_generation,
            },
        );
        Some(DatabaseLoadRequest {
            connection_id: connection_id.to_string(),
            session_generation,
            request_generation,
        })
    }

    pub(crate) fn finish_database_load(
        &mut self,
        request: &DatabaseLoadRequest,
        result: Result<LoadedDatabases, String>,
    ) -> bool {
        if self.session_generation(&request.connection_id) != Some(request.session_generation)
            || !matches!(
                self.databases.get(&request.connection_id),
                Some(DatabaseListState::Loading {
                    session_generation,
                    request_generation,
                }) if *session_generation == request.session_generation
                    && *request_generation == request.request_generation
            )
        {
            return false;
        }

        let state = match result {
            Ok(loaded) if loaded.session_generation == request.session_generation => {
                DatabaseListState::Ready {
                    session_generation: request.session_generation,
                    databases: loaded.databases,
                }
            }
            Ok(_) => return false,
            Err(error) => DatabaseListState::Failed {
                session_generation: request.session_generation,
                error,
            },
        };
        self.databases.insert(request.connection_id.clone(), state);
        true
    }

    pub(crate) fn clear_database_state(&mut self, connection_id: &str) {
        self.databases.remove(connection_id);
    }

    fn next_snapshot_epoch(&mut self) -> u64 {
        self.snapshot_epoch = self
            .snapshot_epoch
            .checked_add(1)
            .expect("workspace snapshot epoch exhausted");
        if let SnapshotState::Ready { refresh, .. } = &mut self.snapshot {
            *refresh = RefreshState::Idle;
        }
        self.snapshot_epoch
    }

    fn apply_snapshot(
        &mut self,
        epoch: u64,
        result: Result<ConnectionWorkspaceSnapshot, ConnectionWorkspaceError>,
    ) -> SnapshotApply {
        if epoch != self.snapshot_epoch {
            return SnapshotApply::Superseded;
        }

        match result {
            Ok(snapshot) => {
                self.replace_snapshot(snapshot);
                SnapshotApply::Applied
            }
            Err(error) => {
                self.snapshot = match std::mem::replace(&mut self.snapshot, SnapshotState::Loading)
                {
                    SnapshotState::Ready { snapshot, .. } => SnapshotState::Ready {
                        snapshot,
                        refresh: RefreshState::Stale(error),
                    },
                    SnapshotState::Loading | SnapshotState::Failed(_) => {
                        SnapshotState::Failed(error)
                    }
                };
                SnapshotApply::Failed
            }
        }
    }

    fn replace_snapshot(&mut self, snapshot: ConnectionWorkspaceSnapshot) {
        self.databases.retain(|connection_id, databases| {
            snapshot.profiles.iter().any(|profile| {
                profile.profile.id == *connection_id
                    && profile.session.generation == Some(databases.session_generation())
            })
        });
        self.operations.retain(|connection_id, _| {
            snapshot
                .profiles
                .iter()
                .any(|profile| profile.profile.id == *connection_id)
        });
        self.snapshot = SnapshotState::Ready {
            snapshot,
            refresh: RefreshState::Idle,
        };
    }

    fn session_generation(&self, connection_id: &str) -> Option<u64> {
        self.snapshot()?
            .profiles
            .iter()
            .find(|profile| profile.profile.id == connection_id)?
            .session
            .generation
    }
}

#[cfg(test)]
mod tests {
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

    fn snapshot(
        revision: i64,
        profiles: Vec<SharedConnectionProfile>,
    ) -> ConnectionWorkspaceSnapshot {
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
}
