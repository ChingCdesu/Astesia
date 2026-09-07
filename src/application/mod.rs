mod atomic_output;
mod catalog_service;
mod chart;
mod connection_service;
pub(crate) mod connection_workspace;
mod connections;
mod document_service;
#[cfg(test)]
mod engine_smoke_tests;
mod er_diagram;
mod export_service;
mod grid_service;
mod grid_transaction;
mod schema_cache;
pub(crate) use grid_transaction::GridTransaction;
mod grid_session;
mod grid_value;
#[cfg(test)]
mod memory_workloads;
mod mutation_service;
mod object_service;
mod performance_dashboard;
mod performance_service;
mod performance_snapshot;
mod profile_editor;
mod query_completion;
mod query_file;
mod query_result_selection;
mod query_service;
mod query_workspace;
mod redis_service;
mod table_structure;
mod transfer;

pub use crate::connection_repository::NativeStateProbe;
pub use catalog_service::CatalogService;
pub(crate) use chart::{ChartDataError, ChartModel, ChartSeries, ChartService, ChartType};
pub use connection_service::{
    ConnectionProfileSnapshot, ConnectionService, ConnectionWorkspaceSnapshot,
    DatabaseSessionSnapshot, LoadedDatabases, ProfileOperationCommand, ProfileOperationCompletion,
    ProfileOperationOutcome,
};
pub(crate) use connection_workspace::{
    CatalogEntry, CatalogKind, CatalogSection, DatabaseCatalogSnapshot,
};
pub use connections::ConnectionOutcome;
pub(crate) use document_service::{DocumentService, DocumentSession, DocumentSessionStatus};
pub(crate) use er_diagram::{
    ErBounds, ErDiagramService, ErDiagramState, ErLayout, ErLoadError, ErPoint, ErSchema, ErStatus,
};
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
    object_creation_policy, object_kind_can_create, object_kind_can_drop, object_kind_can_rename,
    trigger_event_supported, trigger_timing_supported, trigger_uses_function_reference,
    CreateObjectSpec, DatabaseObjectKind, DropObjectTarget, ObjectCreationPolicy, ObjectMutation,
    ObjectMutationError, ObjectService, TableColumnSpec, TriggerEvent, TriggerTiming,
};
pub(crate) use performance_dashboard::{
    PerformanceDashboardState, PerformanceLoadApply, PerformanceRefreshInterval,
};
pub use performance_service::PerformanceService;
pub use performance_snapshot::{
    ClickHouseMetrics, MongoMetrics, MySqlMetrics, PerformanceSnapshot, PostgresMetrics,
    RedisMetrics, SqlServerMetrics, SqliteMetrics,
};
pub(crate) use profile_editor::{
    ProfileDraft, ProfileDraftField, ProfileOrigin, ProfileValidationError, ValidatedProfile,
};
pub(crate) use query_completion::{
    QueryCompletion, QueryCompletionRequest, QueryCompletionService,
};
pub(crate) use query_file::{QueryFileCompletion, QueryFileError, QueryFileRequest};
pub use query_service::QueryService;
pub use query_workspace::{QueryDocument, QueryExecutionScope, QueryTarget};
pub(crate) use query_workspace::{QueryExecutionRequest, QueryOperation, QueryWorkspaceState};
pub(crate) use redis_service::{
    RedisCommand, RedisKeySnapshot, RedisListSide, RedisMutation, RedisPageCursor, RedisService,
    RedisValue,
};
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
use crate::platform::{ProcessSidecarHost, SidecarHostHandle, UiEvent, UiEventBus};
use crate::tasks::TaskManager;
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct Application {
    catalog: CatalogService,
    charts: ChartService,
    connections: ConnectionService,
    documents: DocumentService,
    er_diagrams: ErDiagramService,
    exports: ExportService,
    grids: GridService,
    mutations: MutationService,
    objects: ObjectService,
    performance: PerformanceService,
    query_completions: QueryCompletionService,
    queries: QueryService,
    redis: RedisService,
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
        let repository = SharedConnectionRepository::new_default()?;
        Ok(Self::with_repository_and_sidecar(
            repository,
            Arc::new(ProcessSidecarHost::discover()),
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
        let grids = GridService::new(connection_manager.clone());
        Self {
            catalog,
            charts: ChartService::new(grids.clone()),
            connections: ConnectionService::new(connection_manager.clone(), mcp_registry),
            documents: DocumentService::new(connection_manager.clone()),
            er_diagrams: ErDiagramService::new(connection_manager.clone()),
            exports: ExportService::new(queries.clone(), task_manager.clone()),
            grids,
            mutations: MutationService::new(connection_manager.clone()),
            objects: ObjectService::new(connection_manager.clone()),
            performance: PerformanceService::new(connection_manager.clone()),
            query_completions,
            queries,
            redis: RedisService::new(connection_manager.clone()),
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

    pub(crate) fn charts(&self) -> &ChartService {
        &self.charts
    }

    pub fn exports(&self) -> &ExportService {
        &self.exports
    }

    pub(crate) fn documents(&self) -> &DocumentService {
        &self.documents
    }

    pub(crate) fn er_diagrams(&self) -> &ErDiagramService {
        &self.er_diagrams
    }

    pub fn queries(&self) -> &QueryService {
        &self.queries
    }

    pub(crate) fn redis(&self) -> &RedisService {
        &self.redis
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
