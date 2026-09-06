use std::collections::HashMap;

use super::{ConnectionWorkspaceSnapshot, LoadedDatabases, QueryTarget};
use crate::connection_repository::ConnectionRepositoryError;
use crate::db::{DbType, FunctionInfo, ProcedureInfo, TableInfo, TriggerInfo, UserInfo, ViewInfo};

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObjectLoadRequest {
    connection_id: String,
    database: String,
    session_generation: u64,
    request_generation: u64,
    kind: CatalogKind,
}

impl ObjectLoadRequest {
    pub(crate) const fn kind(&self) -> CatalogKind {
        self.kind
    }

    pub(crate) fn connection_id(&self) -> &str {
        &self.connection_id
    }

    pub(crate) fn database(&self) -> &str {
        &self.database
    }
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

#[derive(Debug, Clone)]
pub(crate) enum ObjectListState {
    Ready {
        session_generation: u64,
        request_generation: u64,
        catalog: DatabaseCatalogSnapshot,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum CatalogSection<T> {
    Unsupported,
    Loading,
    Ready(Vec<T>),
    Failed(String),
}

impl<T> CatalogSection<T> {
    pub(crate) fn from_result(result: Result<Vec<T>, String>) -> Self {
        match result {
            Ok(items) => Self::Ready(items),
            Err(error) => Self::Failed(error),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CatalogKind {
    Schemas,
    Tables,
    Views,
    Functions,
    Procedures,
    Triggers,
    Users,
}

impl CatalogKind {
    pub(crate) const ALL: [Self; 7] = [
        Self::Schemas,
        Self::Tables,
        Self::Views,
        Self::Functions,
        Self::Procedures,
        Self::Triggers,
        Self::Users,
    ];

    pub(crate) fn supported(self, db_type: DbType) -> bool {
        let capabilities = db_type.capabilities();
        match self {
            Self::Tables => true,
            Self::Schemas => capabilities.schemas,
            Self::Views => capabilities.views,
            Self::Functions => capabilities.functions,
            Self::Procedures => capabilities.procedures,
            Self::Triggers => capabilities.triggers,
            Self::Users => capabilities.users,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum CatalogEntry {
    Schemas(CatalogSection<String>),
    Tables(CatalogSection<TableInfo>),
    Views(CatalogSection<ViewInfo>),
    Functions(CatalogSection<FunctionInfo>),
    Procedures(CatalogSection<ProcedureInfo>),
    Triggers(CatalogSection<TriggerInfo>),
    Users(CatalogSection<UserInfo>),
}

impl CatalogEntry {
    pub(crate) const fn kind(&self) -> CatalogKind {
        match self {
            Self::Schemas(_) => CatalogKind::Schemas,
            Self::Tables(_) => CatalogKind::Tables,
            Self::Views(_) => CatalogKind::Views,
            Self::Functions(_) => CatalogKind::Functions,
            Self::Procedures(_) => CatalogKind::Procedures,
            Self::Triggers(_) => CatalogKind::Triggers,
            Self::Users(_) => CatalogKind::Users,
        }
    }

    fn loading(kind: CatalogKind, supported: bool) -> Self {
        match kind {
            CatalogKind::Schemas => Self::Schemas(loading_if(supported)),
            CatalogKind::Tables => Self::Tables(loading_if(supported)),
            CatalogKind::Views => Self::Views(loading_if(supported)),
            CatalogKind::Functions => Self::Functions(loading_if(supported)),
            CatalogKind::Procedures => Self::Procedures(loading_if(supported)),
            CatalogKind::Triggers => Self::Triggers(loading_if(supported)),
            CatalogKind::Users => Self::Users(loading_if(supported)),
        }
    }

    fn is_loading(&self) -> bool {
        matches!(
            self,
            Self::Schemas(CatalogSection::Loading)
                | Self::Tables(CatalogSection::Loading)
                | Self::Views(CatalogSection::Loading)
                | Self::Functions(CatalogSection::Loading)
                | Self::Procedures(CatalogSection::Loading)
                | Self::Triggers(CatalogSection::Loading)
                | Self::Users(CatalogSection::Loading)
        )
    }

    pub(crate) fn failed(kind: CatalogKind, error: impl Into<String>) -> Self {
        let error = error.into();
        match kind {
            CatalogKind::Schemas => Self::Schemas(CatalogSection::Failed(error)),
            CatalogKind::Tables => Self::Tables(CatalogSection::Failed(error)),
            CatalogKind::Views => Self::Views(CatalogSection::Failed(error)),
            CatalogKind::Functions => Self::Functions(CatalogSection::Failed(error)),
            CatalogKind::Procedures => Self::Procedures(CatalogSection::Failed(error)),
            CatalogKind::Triggers => Self::Triggers(CatalogSection::Failed(error)),
            CatalogKind::Users => Self::Users(CatalogSection::Failed(error)),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DatabaseCatalogSnapshot {
    entries: Vec<CatalogEntry>,
}

impl DatabaseCatalogSnapshot {
    pub(crate) fn loading(db_type: DbType) -> Self {
        Self {
            entries: CatalogKind::ALL
                .into_iter()
                .map(|kind| CatalogEntry::loading(kind, kind.supported(db_type)))
                .collect(),
        }
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.iter()
    }

    pub(crate) fn tables(&self) -> &CatalogSection<TableInfo> {
        let CatalogEntry::Tables(section) = self.entry(CatalogKind::Tables) else {
            unreachable!("catalog kind and entry variant must agree")
        };
        section
    }

    pub(crate) fn pending_kinds(&self) -> Vec<CatalogKind> {
        self.entries
            .iter()
            .filter(|entry| entry.is_loading())
            .map(CatalogEntry::kind)
            .collect()
    }

    pub(crate) fn apply(&mut self, entry: CatalogEntry) {
        let kind = entry.kind();
        let current = self
            .entries
            .iter_mut()
            .find(|current| current.kind() == kind)
            .expect("catalog snapshots contain every catalog kind");
        *current = entry;
    }

    fn entry(&self, kind: CatalogKind) -> &CatalogEntry {
        self.entries
            .iter()
            .find(|entry| entry.kind() == kind)
            .expect("catalog snapshots contain every catalog kind")
    }
}

fn loading_if<T>(supported: bool) -> CatalogSection<T> {
    if supported {
        CatalogSection::Loading
    } else {
        CatalogSection::Unsupported
    }
}

impl ObjectListState {
    fn session_generation(&self) -> u64 {
        match self {
            Self::Ready {
                session_generation, ..
            } => *session_generation,
        }
    }

    pub(crate) fn is_loading(&self) -> bool {
        match self {
            Self::Ready { catalog, .. } => !catalog.pending_kinds().is_empty(),
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
    object_request_generation: u64,
    operations: HashMap<String, ProfileOperation>,
    databases: HashMap<String, DatabaseListState>,
    objects: HashMap<(String, String), ObjectListState>,
}

impl Default for ConnectionWorkspaceState {
    fn default() -> Self {
        Self {
            snapshot: SnapshotState::Loading,
            snapshot_epoch: 0,
            operation_generation: 0,
            database_request_generation: 0,
            object_request_generation: 0,
            operations: HashMap::new(),
            databases: HashMap::new(),
            objects: HashMap::new(),
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

    pub(crate) fn objects(&self, target: &QueryTarget) -> Option<&ObjectListState> {
        self.objects
            .get(&(target.connection_id.clone(), target.database.clone()))
    }

    pub(crate) fn query_target_is_live(&self, target: &QueryTarget) -> bool {
        let profile_matches = self.snapshot().is_some_and(|snapshot| {
            snapshot.profiles.iter().any(|profile| {
                profile.profile.id == target.connection_id
                    && profile.profile.db_type == target.db_type
                    && profile.session.generation == Some(target.session_generation)
            })
        });
        let database_matches = matches!(
            self.databases(&target.connection_id),
            Some(DatabaseListState::Ready {
                session_generation,
                databases,
            }) if *session_generation == target.session_generation
                && databases.contains(&target.database)
        );
        profile_matches && database_matches
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
        self.objects
            .retain(|(candidate, _), _| candidate != connection_id);
    }

    pub(crate) fn begin_object_load(
        &mut self,
        target: &QueryTarget,
    ) -> Option<Vec<ObjectLoadRequest>> {
        if !self.query_target_is_live(target) {
            return None;
        }
        let key = (target.connection_id.clone(), target.database.clone());
        if self
            .objects
            .get(&key)
            .is_some_and(|state| state.session_generation() == target.session_generation)
        {
            return None;
        }

        self.object_request_generation = self.object_request_generation.wrapping_add(1);
        let request_generation = self.object_request_generation;
        let catalog = DatabaseCatalogSnapshot::loading(target.db_type);
        let requests = catalog
            .pending_kinds()
            .into_iter()
            .map(|kind| ObjectLoadRequest {
                connection_id: target.connection_id.clone(),
                database: target.database.clone(),
                session_generation: target.session_generation,
                request_generation,
                kind,
            })
            .collect();
        self.objects.insert(
            key,
            ObjectListState::Ready {
                session_generation: target.session_generation,
                request_generation,
                catalog,
            },
        );
        Some(requests)
    }

    pub(crate) fn finish_object_load(
        &mut self,
        request: &ObjectLoadRequest,
        result: CatalogEntry,
    ) -> bool {
        let key = (request.connection_id.clone(), request.database.clone());
        let target_is_live = self.session_generation(&request.connection_id)
            == Some(request.session_generation)
            && matches!(
                self.databases(&request.connection_id),
                Some(DatabaseListState::Ready {
                    session_generation,
                    databases,
                }) if *session_generation == request.session_generation
                    && databases.contains(&request.database)
            );
        if !target_is_live || result.kind() != request.kind {
            return false;
        }
        let Some(ObjectListState::Ready {
            session_generation,
            request_generation,
            catalog,
        }) = self.objects.get_mut(&key)
        else {
            return false;
        };
        if *session_generation != request.session_generation
            || *request_generation != request.request_generation
        {
            return false;
        }
        catalog.apply(result);
        true
    }

    pub(crate) fn clear_object_state(&mut self, target: &QueryTarget) {
        self.objects
            .remove(&(target.connection_id.clone(), target.database.clone()));
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
        self.objects.retain(|(connection_id, database), objects| {
            snapshot.profiles.iter().any(|profile| {
                profile.profile.id == *connection_id
                    && profile.session.generation == Some(objects.session_generation())
                    && matches!(
                        self.databases.get(connection_id),
                        Some(DatabaseListState::Ready {
                            session_generation,
                            databases,
                        }) if *session_generation == objects.session_generation()
                            && databases.contains(database)
                    )
            })
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
mod tests;
