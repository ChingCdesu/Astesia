mod driver;
mod state;

use std::{
    collections::HashMap,
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Weak,
    },
};

use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::db::{create_driver, ConnectionConfig};

pub(crate) use driver::DriverHandle;
pub(crate) use state::ConnectionIntentGeneration;
use state::{ConnectedDriver, ExclusiveInstallation, ReplacingInstallation, RuntimeState};

pub(crate) type ConnectionGeneration = u64;
const MAX_RETAINED_LIFECYCLE_LOCKS: usize = 4_096;

#[derive(Clone)]
pub(crate) struct ConnectionSnapshot<A> {
    handle: DriverHandle,
    pub(super) _attachment: Arc<A>,
    profile_revision: i64,
    generation: ConnectionGeneration,
}

impl<A> ConnectionSnapshot<A> {
    pub(crate) fn handle(&self) -> DriverHandle {
        self.handle.clone()
    }

    pub(crate) fn profile_revision(&self) -> i64 {
        self.profile_revision
    }

    pub(crate) fn generation(&self) -> ConnectionGeneration {
        self.generation
    }
}

#[derive(Debug)]
pub(crate) enum ReplacingConnectError<E> {
    Connect(String),
    RevisionChanged,
    Verification(E),
    Superseded,
}

#[derive(Debug)]
pub(crate) enum ExclusiveConnectError<E> {
    Connect(String),
    RevisionChanged,
    Verification(E),
    Occupied(ConnectionGeneration),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExclusiveConnectOutcome {
    Opened,
    Existing,
}

pub(crate) struct RuntimeDisconnectOutcome {
    pub generation: Option<ConnectionGeneration>,
    pub result: Result<bool, String>,
}

pub(crate) struct ConnectionRuntime<A> {
    state: Arc<Mutex<RuntimeState<A>>>,
    global_lifecycle: Arc<Mutex<()>>,
    connection_lifecycles: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
    next_generation: Arc<AtomicU64>,
}

impl<A> Clone for ConnectionRuntime<A> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            global_lifecycle: self.global_lifecycle.clone(),
            connection_lifecycles: self.connection_lifecycles.clone(),
            next_generation: self.next_generation.clone(),
        }
    }
}

impl<A> Default for ConnectionRuntime<A> {
    fn default() -> Self {
        Self {
            state: Arc::default(),
            global_lifecycle: Arc::default(),
            connection_lifecycles: Arc::default(),
            next_generation: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl<A> ConnectionRuntime<A> {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn allocate_generation(&self) -> ConnectionGeneration {
        loop {
            let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
            if generation != 0 {
                return generation;
            }
        }
    }

    pub(crate) async fn lock_connection_lifecycle(
        &self,
        connection_id: &str,
    ) -> OwnedMutexGuard<()> {
        let lifecycle = {
            let mut lifecycles = self.connection_lifecycles.lock().await;
            if lifecycles.len() >= MAX_RETAINED_LIFECYCLE_LOCKS {
                lifecycles.retain(|_, lifecycle| lifecycle.strong_count() > 0);
            }
            if let Some(lifecycle) = lifecycles.get(connection_id).and_then(Weak::upgrade) {
                lifecycle
            } else {
                let lifecycle = Arc::new(Mutex::new(()));
                lifecycles.insert(connection_id.to_string(), Arc::downgrade(&lifecycle));
                lifecycle
            }
        };
        lifecycle.lock_owned().await
    }

    pub(crate) async fn lock_global_lifecycle(&self) -> OwnedMutexGuard<()> {
        self.global_lifecycle.clone().lock_owned().await
    }

    pub(crate) async fn begin_replacing_intent(
        &self,
        connection_id: &str,
    ) -> ConnectionIntentGeneration {
        let _lifecycle = self.lock_global_lifecycle().await;
        self.state.lock().await.advance_intent(connection_id)
    }

    pub(crate) async fn connect_replacing<E, Verify, VerifyFuture>(
        &self,
        connection_id: String,
        intent: ConnectionIntentGeneration,
        config: ConnectionConfig,
        profile_revision: i64,
        attachment: A,
        verify: Verify,
    ) -> Result<(), ReplacingConnectError<E>>
    where
        Verify: FnOnce() -> VerifyFuture,
        VerifyFuture: Future<Output = Result<i64, E>>,
    {
        let mut driver = create_driver(&config);
        if let Err(error) = driver.connect().await {
            return Err(ReplacingConnectError::Connect(error.to_string()));
        }

        let lifecycle = self.lock_global_lifecycle().await;
        let latest_revision = match verify().await {
            Ok(revision) => revision,
            Err(error) => {
                drop(lifecycle);
                let _ = driver.disconnect().await;
                return Err(ReplacingConnectError::Verification(error));
            }
        };
        if latest_revision != profile_revision {
            drop(lifecycle);
            let _ = driver.disconnect().await;
            return Err(ReplacingConnectError::RevisionChanged);
        }

        let candidate = DriverHandle::new(driver);
        let installation = self.state.lock().await.install_replacing_if_current(
            connection_id.clone(),
            intent,
            ConnectedDriver {
                handle: candidate.clone(),
                attachment: Arc::new(attachment),
                profile_revision,
                generation: intent.0,
            },
        );
        drop(lifecycle);

        match installation {
            ReplacingInstallation::Installed { replaced } => {
                if let Some(replaced) = replaced {
                    let _ = replaced.handle.disconnect().await;
                    drop(replaced.attachment);
                }
            }
            ReplacingInstallation::Superseded { discarded } => {
                let _ = discarded.handle.disconnect().await;
                drop(discarded.attachment);
                return Err(ReplacingConnectError::Superseded);
            }
        }

        if !self
            .state
            .lock()
            .await
            .is_current_driver(&connection_id, &candidate)
        {
            return Err(ReplacingConnectError::Superseded);
        }
        Ok(())
    }

    pub(crate) async fn connect_exclusive<E, Verify, VerifyFuture>(
        &self,
        connection_id: String,
        generation: ConnectionGeneration,
        config: ConnectionConfig,
        profile_revision: i64,
        attachment: A,
        verify: Verify,
    ) -> Result<ExclusiveConnectOutcome, ExclusiveConnectError<E>>
    where
        Verify: FnOnce() -> VerifyFuture,
        VerifyFuture: Future<Output = Result<i64, E>>,
    {
        let mut driver = create_driver(&config);
        if let Err(error) = driver.connect().await {
            return Err(ExclusiveConnectError::Connect(error.to_string()));
        }

        let latest_revision = match verify().await {
            Ok(revision) => revision,
            Err(error) => {
                let _ = driver.disconnect().await;
                return Err(ExclusiveConnectError::Verification(error));
            }
        };
        if latest_revision != profile_revision {
            let _ = driver.disconnect().await;
            return Err(ExclusiveConnectError::RevisionChanged);
        }

        let installation = self.state.lock().await.install_exclusive(
            connection_id,
            ConnectedDriver {
                handle: DriverHandle::new(driver),
                attachment: Arc::new(attachment),
                profile_revision,
                generation,
            },
        );
        match installation {
            ExclusiveInstallation::Installed => Ok(ExclusiveConnectOutcome::Opened),
            ExclusiveInstallation::ExistingSameGeneration { discarded } => {
                let _ = discarded.handle.disconnect().await;
                drop(discarded.attachment);
                Ok(ExclusiveConnectOutcome::Existing)
            }
            ExclusiveInstallation::Occupied {
                existing_generation,
                discarded,
            } => {
                let _ = discarded.handle.disconnect().await;
                drop(discarded.attachment);
                Err(ExclusiveConnectError::Occupied(existing_generation))
            }
        }
    }

    pub(crate) async fn connection(&self, connection_id: &str) -> Option<ConnectionSnapshot<A>> {
        self.state.lock().await.connection(connection_id)
    }

    pub(crate) async fn connected_generation(
        &self,
        connection_id: &str,
    ) -> Option<ConnectionGeneration> {
        self.connection(connection_id)
            .await
            .map(|connection| connection.generation())
    }

    pub(crate) async fn driver(&self, connection_id: &str) -> Option<DriverHandle> {
        self.state.lock().await.driver(connection_id)
    }

    pub(crate) async fn driver_pair(
        &self,
        source_connection_id: &str,
        target_connection_id: &str,
    ) -> (Option<DriverHandle>, Option<DriverHandle>) {
        let state = self.state.lock().await;
        (
            state.driver(source_connection_id),
            state.driver(target_connection_id),
        )
    }

    pub(crate) async fn driver_session(
        &self,
        connection_id: &str,
    ) -> Option<(DriverHandle, ConnectionGeneration)> {
        self.state.lock().await.driver_session(connection_id)
    }

    pub(crate) async fn disconnect_replacing_under_global(
        &self,
        lifecycle: OwnedMutexGuard<()>,
        connection_id: &str,
    ) -> bool {
        let driver = self.state.lock().await.invalidate_and_detach(connection_id);
        drop(lifecycle);
        disconnect_entry(driver).await.result.unwrap_or(false)
    }

    pub(crate) async fn reconcile_revisions_under_global(
        &self,
        lifecycle: OwnedMutexGuard<()>,
        revisions: HashMap<String, i64>,
    ) -> HashMap<String, ConnectionGeneration> {
        let (drivers, generations) = {
            let mut state = self.state.lock().await;
            let drivers = state.detach_stale(&revisions);
            let generations = state.session_generations();
            (drivers, generations)
        };
        drop(lifecycle);
        for driver in drivers {
            let _ = driver.handle.disconnect().await;
            drop(driver.attachment);
        }
        generations
    }

    pub(crate) async fn disconnect(&self, connection_id: &str) -> RuntimeDisconnectOutcome {
        let driver = self.state.lock().await.detach(connection_id);
        disconnect_entry(driver).await
    }

    pub(crate) async fn disconnect_if_generation(
        &self,
        connection_id: &str,
        generation: ConnectionGeneration,
    ) -> Result<bool, String> {
        let driver = self
            .state
            .lock()
            .await
            .detach_if_generation(connection_id, generation);
        disconnect_entry(driver).await.result
    }

    #[cfg(test)]
    pub(crate) async fn contains(&self, connection_id: &str) -> bool {
        self.state.lock().await.connection(connection_id).is_some()
    }
}

async fn disconnect_entry<A>(driver: Option<ConnectedDriver<A>>) -> RuntimeDisconnectOutcome {
    let Some(driver) = driver else {
        return RuntimeDisconnectOutcome {
            generation: None,
            result: Ok(false),
        };
    };
    let generation = driver.generation;
    let result = driver.handle.disconnect().await.map(|_| true);
    drop(driver.attachment);
    RuntimeDisconnectOutcome {
        generation: Some(generation),
        result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ConnectionConfig, DbType};

    fn sqlite_config(id: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: id.to_string(),
            name: id.to_string(),
            db_type: DbType::SQLite,
            host: ":memory:".to_string(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: None,
            color: None,
        }
    }

    #[tokio::test]
    async fn newer_replacing_intent_supersedes_an_older_installation() {
        let runtime = ConnectionRuntime::new();
        let older = runtime.begin_replacing_intent("local").await;
        let newer = runtime.begin_replacing_intent("local").await;

        runtime
            .connect_replacing(
                "local".into(),
                newer,
                sqlite_config("local"),
                1,
                (),
                || async { Ok::<_, ()>(1) },
            )
            .await
            .expect("newer intent");
        assert!(matches!(
            runtime
                .connect_replacing(
                    "local".into(),
                    older,
                    sqlite_config("local"),
                    1,
                    (),
                    || async { Ok::<_, ()>(1) }
                )
                .await,
            Err(ReplacingConnectError::Superseded)
        ));
        assert_eq!(runtime.connected_generation("local").await, Some(newer.0));
    }

    #[tokio::test]
    async fn exclusive_generation_is_idempotent_but_rejects_another_owner() {
        let runtime = ConnectionRuntime::new();
        assert_eq!(
            runtime
                .connect_exclusive(
                    "local".into(),
                    41,
                    sqlite_config("local"),
                    1,
                    (),
                    || async { Ok::<_, ()>(1) }
                )
                .await
                .expect("first"),
            ExclusiveConnectOutcome::Opened
        );
        assert_eq!(
            runtime
                .connect_exclusive(
                    "local".into(),
                    41,
                    sqlite_config("local"),
                    1,
                    (),
                    || async { Ok::<_, ()>(1) }
                )
                .await
                .expect("same generation"),
            ExclusiveConnectOutcome::Existing
        );
        assert!(matches!(
            runtime
                .connect_exclusive(
                    "local".into(),
                    42,
                    sqlite_config("local"),
                    1,
                    (),
                    || async { Ok::<_, ()>(1) }
                )
                .await,
            Err(ExclusiveConnectError::Occupied(41))
        ));
    }
}
