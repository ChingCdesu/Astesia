mod catalog_service;
mod connection_service;
pub(crate) mod connection_workspace;
mod connections;
#[cfg(test)]
mod engine_smoke_tests;
mod export_service;
mod grid_service;
mod grid_session;
mod grid_value;
mod mutation_service;
mod object_service;
mod performance_service;
mod performance_snapshot;
mod profile_editor;
mod query_completion;
mod query_file;
mod query_result_selection;
mod query_service;
mod query_workspace;
mod table_structure;
mod transfer;

pub use crate::connection_repository::NativeStateProbe;
pub use catalog_service::CatalogService;
pub use connection_service::{
    ConnectionProfileSnapshot, ConnectionService, ConnectionWorkspaceSnapshot,
    DatabaseSessionSnapshot, LoadedDatabases, ProfileOperationCommand, ProfileOperationCompletion,
    ProfileOperationOutcome,
};
pub(crate) use connection_workspace::{
    CatalogKind, CatalogLoadResult, CatalogSection, DatabaseCatalogSnapshot,
};
pub use connections::ConnectionOutcome;
pub use export_service::{
    CsvOptions, ExportFormat, ExportService, ExportSource, JsonLayout, JsonOptions, XlsxOptions,
};
pub(crate) use grid_service::{GridLoadError, GridSaveOutcome, GridService};
pub(crate) use grid_session::{
    GridCell, GridCellSelection, GridChangeSummary, GridDelete, GridDraftRow, GridEditability,
    GridInsert, GridLoadRequest, GridPage, GridQuery, GridRowSelectionMode, GridSaveFailure,
    GridSavePlan, GridSaveRequest, GridSession, GridSessionError, GridSessionStatus, GridSort,
    GridSortDirection, GridUpdate, DEFAULT_GRID_PAGE_SIZE,
};
pub(crate) use grid_value::{GridCellInputError, GridColumn, GridColumnKind};
pub use mutation_service::{MutationService, RowUpdate};
pub(crate) use object_service::{
    object_kind_can_create, object_kind_can_drop, object_kind_can_rename, CreateObjectSpec,
    DatabaseObjectKind, DropObjectTarget, ObjectMutation, ObjectMutationError, ObjectService,
    TableColumnSpec, TriggerEvent, TriggerTiming,
};
pub use performance_service::PerformanceService;
pub use performance_snapshot::{
    ClickHouseMetrics, MySqlMetrics, PerformanceSnapshot, PostgresMetrics, RedisMetrics,
    SqlServerMetrics, SqliteMetrics,
};
pub(crate) use profile_editor::{ProfileDraft, ProfileDraftField, ProfileOrigin, ValidatedProfile};
pub(crate) use query_completion::{
    QueryCompletion, QueryCompletionRequest, QueryCompletionService,
};
pub(crate) use query_file::{QueryFileCompletion, QueryFileError, QueryFileRequest};
pub use query_service::QueryService;
pub use query_workspace::{QueryDocument, QueryExecutionScope, QueryTarget};
pub(crate) use query_workspace::{QueryExecutionRequest, QueryOperation, QueryWorkspaceState};
pub(crate) use table_structure::{
    TableStructureLoadError, TableStructureSnapshot, TableStructureState, TableStructureStatus,
};
pub use transfer::{
    BackupContent, BackupOptions, CopyContent, CopyOptions, DropTableMode, TransferService,
};

use crate::connection_repository::{
    probe_default_native_state, ConnectionRepositoryError, SharedConnectionRepository,
};
use crate::mcp_runtime::McpRuntime;
use crate::mcp_sync_server::McpSyncRegistry;
use crate::platform::{SidecarHostHandle, UiEvent, UiEventBus};
use crate::tasks::TaskManager;
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct Application {
    catalog: CatalogService,
    connections: ConnectionService,
    exports: ExportService,
    grids: GridService,
    mutations: MutationService,
    objects: ObjectService,
    performance: PerformanceService,
    query_completions: QueryCompletionService,
    queries: QueryService,
    transfers: TransferService,
    mcp: Option<McpRuntime>,
    task_manager: TaskManager,
    events: UiEventBus,
}

impl Application {
    pub async fn probe_native_state() -> Result<NativeStateProbe, ConnectionRepositoryError> {
        probe_default_native_state().await
    }

    pub fn new() -> Result<Self, ConnectionRepositoryError> {
        Ok(Self::with_repository(
            SharedConnectionRepository::new_default()?,
        ))
    }

    pub fn with_repository(repository: SharedConnectionRepository) -> Self {
        Self::compose(repository, None)
    }

    pub fn with_repository_and_sidecar(
        repository: SharedConnectionRepository,
        sidecar_host: SidecarHostHandle,
    ) -> Self {
        Self::compose(repository, Some(sidecar_host))
    }

    fn compose(
        repository: SharedConnectionRepository,
        sidecar_host: Option<SidecarHostHandle>,
    ) -> Self {
        let events = UiEventBus::new();
        let connection_manager = connections::ConnectionManager::new(repository.clone());
        let mcp_registry = McpSyncRegistry::new(repository, Arc::new(events.clone()));
        let task_manager = TaskManager::new(Arc::new(events.clone()));
        let queries = QueryService::new(connection_manager.clone());
        let catalog = CatalogService::new(connection_manager.clone());
        let query_completions = QueryCompletionService::new(catalog.clone());
        let mcp = sidecar_host.map(|host| McpRuntime::new(host, mcp_registry.clone()));
        Self {
            catalog,
            connections: ConnectionService::new(connection_manager.clone(), mcp_registry),
            exports: ExportService::new(queries.clone()),
            grids: GridService::new(connection_manager.clone()),
            mutations: MutationService::new(connection_manager.clone()),
            objects: ObjectService::new(connection_manager.clone()),
            performance: PerformanceService::new(connection_manager.clone()),
            query_completions,
            queries,
            transfers: TransferService::new(connection_manager, task_manager.clone()),
            mcp,
            task_manager,
            events,
        }
    }

    pub fn connections(&self) -> &ConnectionService {
        &self.connections
    }

    pub fn catalog(&self) -> &CatalogService {
        &self.catalog
    }

    pub fn exports(&self) -> &ExportService {
        &self.exports
    }

    pub fn queries(&self) -> &QueryService {
        &self.queries
    }

    pub(crate) fn grids(&self) -> &GridService {
        &self.grids
    }

    pub(crate) fn query_completions(&self) -> &QueryCompletionService {
        &self.query_completions
    }

    pub fn mutations(&self) -> &MutationService {
        &self.mutations
    }

    pub(crate) fn objects(&self) -> &ObjectService {
        &self.objects
    }

    pub fn performance(&self) -> &PerformanceService {
        &self.performance
    }

    pub fn transfers(&self) -> &TransferService {
        &self.transfers
    }

    pub fn mcp(&self) -> Option<&McpRuntime> {
        self.mcp.as_ref()
    }

    pub fn tasks(&self) -> &TaskManager {
        &self.task_manager
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<UiEvent> {
        self.events.subscribe()
    }
}
