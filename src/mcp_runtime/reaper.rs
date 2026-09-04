use std::sync::{Arc, Mutex, MutexGuard};

use crate::platform::SidecarControlHandle;

#[derive(Clone, Default)]
pub(super) struct ProcessReaper {
    pending: Arc<Mutex<Vec<SidecarControlHandle>>>,
}

impl ProcessReaper {
    pub(super) fn retain(&self, control: SidecarControlHandle) {
        let mut pending = lock_unpoisoned(&self.pending);
        if !pending
            .iter()
            .any(|existing| Arc::ptr_eq(existing, &control))
        {
            pending.push(control);
        }
    }

    pub(super) fn retry(&self) -> Vec<String> {
        let pending = std::mem::take(&mut *lock_unpoisoned(&self.pending));
        let mut failed = Vec::new();
        let mut errors = Vec::new();
        for control in pending {
            if let Err(error) = control.terminate() {
                failed.push(control);
                errors.push(error);
            }
        }
        lock_unpoisoned(&self.pending).extend(failed);
        errors
    }

    #[cfg(test)]
    pub(super) fn pending_len(&self) -> usize {
        lock_unpoisoned(&self.pending).len()
    }
}

pub(super) fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
