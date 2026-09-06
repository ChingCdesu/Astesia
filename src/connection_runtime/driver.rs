use std::{
    ops::Deref,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use tokio::sync::{Mutex, MutexGuard};

use crate::db::DatabaseDriver;

#[derive(Clone)]
pub(crate) struct DriverHandle {
    inner: Arc<DriverHandleInner>,
}

struct DriverHandleInner {
    retired: AtomicBool,
    retirement: tokio::sync::watch::Sender<bool>,
    disconnect_started: AtomicBool,
    driver: Mutex<Box<dyn DatabaseDriver>>,
}

pub(crate) struct DriverGuard<'a> {
    guard: MutexGuard<'a, Box<dyn DatabaseDriver>>,
}

impl DriverHandle {
    pub(crate) fn new(driver: Box<dyn DatabaseDriver>) -> Self {
        Self {
            inner: Arc::new(DriverHandleInner {
                retired: AtomicBool::new(false),
                retirement: tokio::sync::watch::channel(false).0,
                disconnect_started: AtomicBool::new(false),
                driver: Mutex::new(driver),
            }),
        }
    }

    pub(crate) async fn lock_active(&self) -> Result<DriverGuard<'_>, String> {
        let guard = self.inner.driver.lock().await;
        if self.inner.retired.load(Ordering::Acquire) {
            return Err("连接已断开".to_string());
        }
        Ok(DriverGuard { guard })
    }

    pub(super) fn retire(&self) {
        self.inner.retired.store(true, Ordering::Release);
        self.inner.retirement.send_replace(true);
    }

    pub(crate) fn retirement(&self) -> tokio::sync::watch::Receiver<bool> {
        self.inner.retirement.subscribe()
    }

    pub(super) fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub(super) async fn disconnect(&self) -> Result<(), String> {
        self.retire();
        if self.inner.disconnect_started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let mut driver = self.inner.driver.lock().await;
        driver.disconnect().await.map_err(|error| error.to_string())
    }
}

impl Deref for DriverGuard<'_> {
    type Target = dyn DatabaseDriver;

    fn deref(&self) -> &Self::Target {
        self.guard.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::db::{create_driver, ConnectionConfig, DbType};

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
    async fn retired_handle_rejects_work_waiting_on_the_driver() {
        let handle = DriverHandle::new(create_driver(&sqlite_config("retired")));
        let active = handle.lock_active().await.expect("active driver");
        let waiting_handle = handle.clone();
        let waiting =
            tokio::spawn(async move { waiting_handle.lock_active().await.map(|_driver| ()) });
        tokio::task::yield_now().await;

        handle.retire();
        drop(active);

        let error = tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("waiting operation completed")
            .expect("waiting task")
            .expect_err("retired handle must reject queued work");
        assert_eq!(error, "连接已断开");

        handle.disconnect().await.expect("disconnect");
        assert_eq!(
            handle
                .lock_active()
                .await
                .err()
                .expect("disconnected handle must stay retired"),
            "连接已断开"
        );
    }

    #[tokio::test]
    async fn different_driver_handles_do_not_share_an_io_lock() {
        let first = DriverHandle::new(create_driver(&sqlite_config("first")));
        let second = DriverHandle::new(create_driver(&sqlite_config("second")));
        let first_guard = first.lock_active().await.expect("first driver");

        let waiting_first = first.clone();
        let waiting = tokio::spawn(async move {
            waiting_first
                .lock_active()
                .await
                .map(|driver| driver.db_type())
        });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());

        let second_guard = tokio::time::timeout(Duration::from_secs(1), second.lock_active())
            .await
            .expect("second handle must not wait for the first")
            .expect("second driver");
        assert_eq!(second_guard.db_type(), DbType::SQLite);
        drop(second_guard);

        drop(first_guard);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), waiting)
                .await
                .expect("same-handle waiter completed")
                .expect("same-handle task")
                .expect("same-handle driver"),
            DbType::SQLite
        );
    }
}
