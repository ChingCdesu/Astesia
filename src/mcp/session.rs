use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex, Weak},
    time::Duration,
};

use async_trait::async_trait;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{
    connection_repository::SharedConnectionRepository,
    mcp_sync::{McpControlCommand, McpSyncClient},
};

use super::catalog::{Catalog, CatalogError};

#[derive(Clone)]
pub struct AstesiaMcp {
    pub(super) active_tests: ActiveConnectionTests,
    pub(super) catalog: Catalog,
    pub(super) sync: Option<McpSyncClient>,
    _control_loop: Option<Arc<ControlLoopTask>>,
}

#[derive(Clone, Default)]
pub(super) struct ActiveConnectionTests {
    inner: Arc<StdMutex<HashMap<String, Weak<ActiveConnectionTest>>>>,
}

pub(super) struct ActiveConnectionTest {
    pub(super) generation: u64,
    pub(super) owns_sync_ownership: bool,
    pub(super) cancellation: CancellationToken,
    future_dropped: CancellationToken,
}

impl ActiveConnectionTest {
    fn new(generation: u64, owns_sync_ownership: bool) -> Self {
        Self {
            generation,
            owns_sync_ownership,
            cancellation: CancellationToken::new(),
            future_dropped: CancellationToken::new(),
        }
    }

    pub(super) fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub(super) async fn wait_until_future_dropped(&self) {
        self.future_dropped.cancelled().await;
    }

    pub(super) fn mark_future_dropped(&self) {
        self.future_dropped.cancel();
    }
}

impl ActiveConnectionTests {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Weak<ActiveConnectionTest>>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn current(&self, connection_id: &str) -> Option<Arc<ActiveConnectionTest>> {
        let mut active = self.lock();
        let current = active.get(connection_id).and_then(Weak::upgrade);
        if current.is_none() {
            active.remove(connection_id);
        }
        current
    }

    pub(super) fn register(
        &self,
        connection_id: &str,
        generation: u64,
        owns_sync_ownership: bool,
        pending_sync_lease: Option<PendingSyncLease>,
    ) -> Result<ActiveConnectionTestGuard, String> {
        let test = Arc::new(ActiveConnectionTest::new(generation, owns_sync_ownership));
        let pending_sync_lease = pending_sync_lease
            .map(|lease| lease.with_cleanup(PendingLeaseCleanup::Test(test.clone())));
        let mut active = self.lock();
        if active.get(connection_id).and_then(Weak::upgrade).is_some() {
            return Err(format!(
                "连接 {connection_id} 已有并发测试，请等待其完成后重试"
            ));
        }
        active.insert(connection_id.to_string(), Arc::downgrade(&test));
        Ok(ActiveConnectionTestGuard {
            test,
            pending_sync_lease,
        })
    }
}

#[derive(Clone)]
pub(super) struct ActiveTestMarker {
    _state: Arc<ActiveConnectionTest>,
}

impl ActiveTestMarker {
    pub(super) fn new(state: Arc<ActiveConnectionTest>) -> Self {
        Self { _state: state }
    }
}

#[async_trait]
pub(super) trait SyncLeaseClient: Send + Sync {
    async fn connected(&self, connection_id: String, generation: u64) -> Result<(), String>;
    async fn released(&self, connection_id: String, generation: u64) -> Result<(), String>;
}

#[async_trait]
impl SyncLeaseClient for McpSyncClient {
    async fn connected(&self, connection_id: String, generation: u64) -> Result<(), String> {
        McpSyncClient::connected(self, connection_id, generation)
            .await
            .map_err(|error| error.to_string())
    }

    async fn released(&self, connection_id: String, generation: u64) -> Result<(), String> {
        McpSyncClient::released(self, connection_id, generation)
            .await
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone)]
enum PendingLeaseCleanup {
    None,
    Test(Arc<ActiveConnectionTest>),
    Connection(Catalog),
}

pub(super) struct PendingSyncLease {
    sync: Arc<dyn SyncLeaseClient>,
    cleanup: PendingLeaseCleanup,
    connection_id: String,
    generation: u64,
    pending: bool,
}

impl PendingSyncLease {
    pub(super) fn new(
        sync: impl SyncLeaseClient + 'static,
        connection_id: String,
        generation: u64,
    ) -> Self {
        Self {
            sync: Arc::new(sync),
            cleanup: PendingLeaseCleanup::None,
            connection_id,
            generation,
            pending: true,
        }
    }

    #[cfg(test)]
    pub(super) fn with_client(
        sync: Arc<dyn SyncLeaseClient>,
        connection_id: String,
        generation: u64,
    ) -> Self {
        Self {
            sync,
            cleanup: PendingLeaseCleanup::None,
            connection_id,
            generation,
            pending: true,
        }
    }

    fn with_cleanup(mut self, cleanup: PendingLeaseCleanup) -> Self {
        self.cleanup = cleanup;
        self
    }

    pub(super) fn for_connection(mut self, catalog: Catalog) -> Self {
        self.cleanup = PendingLeaseCleanup::Connection(catalog);
        self
    }

    pub(super) const fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) async fn commit_connected(&mut self) -> Result<(), String> {
        self.sync
            .connected(self.connection_id.clone(), self.generation)
            .await?;
        self.pending = false;
        Ok(())
    }

    pub(super) async fn release(&mut self) -> Result<(), String> {
        self.sync
            .released(self.connection_id.clone(), self.generation)
            .await?;
        self.pending = false;
        Ok(())
    }
}

impl Drop for PendingSyncLease {
    fn drop(&mut self) {
        if !self.pending {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let sync = self.sync.clone();
        let connection_id = self.connection_id.clone();
        let generation = self.generation;
        let cleanup = self.cleanup.clone();
        runtime.spawn(async move {
            let is_test = matches!(&cleanup, PendingLeaseCleanup::Test(_));
            let _marker = match cleanup {
                PendingLeaseCleanup::None => None,
                PendingLeaseCleanup::Test(test) => Some(ActiveTestMarker::new(test)),
                PendingLeaseCleanup::Connection(catalog) => {
                    let _lifecycle = catalog.lock_connection_lifecycle(&connection_id).await;
                    if let Err(error) = catalog
                        .disconnect_if_generation_under_lifecycle(&connection_id, generation)
                        .await
                    {
                        log::debug!(
                            "Unable to roll back an abandoned MCP connection generation: {error}"
                        );
                    }
                    None
                }
            };
            if let Err(error) = sync.released(connection_id, generation).await {
                if is_test {
                    log::debug!(
                        "Unable to release an abandoned HTTP connection test generation: {error}"
                    );
                } else {
                    log::debug!(
                        "Unable to release an abandoned HTTP connection generation: {error}"
                    );
                }
            }
        });
    }
}

pub(super) struct ActiveConnectionTestGuard {
    pub(super) test: Arc<ActiveConnectionTest>,
    pending_sync_lease: Option<PendingSyncLease>,
}

impl std::ops::Deref for ActiveConnectionTestGuard {
    type Target = ActiveConnectionTest;

    fn deref(&self) -> &Self::Target {
        &self.test
    }
}

impl ActiveConnectionTestGuard {
    pub(super) async fn release_pending_sync_lease(&mut self) -> Result<(), String> {
        if let Some(pending_sync_lease) = self.pending_sync_lease.as_mut() {
            pending_sync_lease.release().await?;
        }
        Ok(())
    }
}

impl Drop for ActiveConnectionTestGuard {
    fn drop(&mut self) {
        // Dropping an MCP request future must always tell App controls that
        // the database test future (and its shared OS lease) is gone.
        self.test.mark_future_dropped();
    }
}

struct ControlLoopTask {
    handle: JoinHandle<()>,
}

impl Drop for ControlLoopTask {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn handle_control_command(
    catalog: &Catalog,
    active_tests: &ActiveConnectionTests,
    command: &McpControlCommand,
) -> Result<bool, CatalogError> {
    // This lock linearizes App controls with HTTP test Acquire + registration.
    // Once the control owns it, an acquired test is either registered here or
    // has already dropped its future and lease.
    let _lifecycle = catalog
        .lock_connection_lifecycle(&command.connection_id)
        .await;
    let active_test = active_tests.current(&command.connection_id);
    if let Some(active_test) =
        active_test.filter(|active_test| active_test.generation == command.generation)
    {
        active_test.cancel();
        active_test.wait_until_future_dropped().await;
        if active_test.owns_sync_ownership {
            // The test owns this generation and has no persistent driver. Its
            // Released request may still be in flight; ACK is safe now because
            // the test future and cross-process shared lease are already gone.
            return Ok(true);
        }
    }
    catalog
        .disconnect_if_generation_under_lifecycle(&command.connection_id, command.generation)
        .await
}

impl AstesiaMcp {
    pub(super) fn with_repository(repository: SharedConnectionRepository) -> Self {
        Self {
            catalog: Catalog::with_repository(repository),
            active_tests: ActiveConnectionTests::default(),
            sync: None,
            _control_loop: None,
        }
    }

    pub(super) fn with_repository_and_sync(
        repository: SharedConnectionRepository,
        sync: McpSyncClient,
    ) -> Self {
        let catalog = Catalog::with_repository(repository);
        let active_tests = ActiveConnectionTests::default();
        let loop_catalog = catalog.clone();
        let loop_sync = sync.clone();
        let loop_active_tests = active_tests.clone();
        let handle = tokio::spawn(async move {
            loop {
                match loop_sync.poll_control().await {
                    Ok(Some(command)) => {
                        let result =
                            handle_control_command(&loop_catalog, &loop_active_tests, &command)
                                .await;
                        let (ok, error) = match result {
                            Ok(_) => (true, None),
                            Err(error) => (false, Some(error.to_string())),
                        };
                        if let Err(report_error) =
                            loop_sync.control_result(&command, ok, error).await
                        {
                            log::warn!(
                                "Unable to report an App-requested MCP disconnect: {report_error}"
                            );
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        log::warn!("Unable to poll Astesia MCP control commands: {error}");
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                }
            }
        });
        Self {
            catalog,
            active_tests,
            sync: Some(sync),
            _control_loop: Some(Arc::new(ControlLoopTask { handle })),
        }
    }
}
