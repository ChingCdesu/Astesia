mod backup;
mod restore;
mod table_copy;

pub use backup::{BackupOptions, DropTableMode};
pub use table_copy::CopyOptions;

pub type BackupContent = TransferContent;
pub type CopyContent = TransferContent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferContent {
    Structure,
    Data,
    StructureAndData,
}

impl TransferContent {
    const fn includes_structure(self) -> bool {
        matches!(self, Self::Structure | Self::StructureAndData)
    }

    const fn includes_data(self) -> bool {
        matches!(self, Self::Data | Self::StructureAndData)
    }

    const fn step_count(self) -> usize {
        match self {
            Self::Structure | Self::Data => 1,
            Self::StructureAndData => 2,
        }
    }
}

pub(super) enum TransferFailure {
    Cancelled(String),
    Failed(String),
}

impl TransferFailure {
    pub(super) fn cancelled(message: impl Into<String>) -> Self {
        Self::Cancelled(message.into())
    }

    pub(super) fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }

    fn into_outcome(self) -> TaskOutcome {
        match self {
            Self::Cancelled(message) => TaskOutcome::Cancelled(message),
            Self::Failed(message) => TaskOutcome::Failed(message),
        }
    }
}

#[derive(Default)]
pub(super) struct TransferEffects {
    applied_changes: u64,
    failed_changes: u64,
    failure_details: Vec<String>,
}

impl TransferEffects {
    pub(super) fn record_applied(&mut self, count: u64) {
        self.applied_changes = self.applied_changes.saturating_add(count);
    }

    pub(super) fn record_failure(&mut self, count: u64, detail: impl Into<String>) {
        if count == 0 {
            return;
        }
        self.failed_changes = self.failed_changes.saturating_add(count);
        let detail = detail.into();
        if !self.failure_details.contains(&detail) {
            self.failure_details.push(detail);
        }
    }

    pub(super) const fn failure_count(&self) -> u64 {
        self.failed_changes
    }

    pub(super) fn finish(
        self,
        operation: &str,
        completed_message: impl FnOnce(u64) -> String,
    ) -> TaskOutcome {
        if self.failed_changes == 0 {
            return TaskOutcome::Completed(completed_message(self.applied_changes));
        }
        self.incomplete_outcome(operation, None)
    }

    pub(super) fn interrupted(self, operation: &str, failure: TransferFailure) -> TaskOutcome {
        if self.applied_changes == 0 && self.failed_changes == 0 {
            return failure.into_outcome();
        }
        let reason = match failure {
            TransferFailure::Cancelled(message) | TransferFailure::Failed(message) => message,
        };
        self.incomplete_outcome(operation, Some(&reason))
    }

    fn incomplete_outcome(self, operation: &str, terminal_reason: Option<&str>) -> TaskOutcome {
        let message = self.incomplete_message(operation, terminal_reason);
        if self.applied_changes == 0 {
            TaskOutcome::Failed(message)
        } else {
            TaskOutcome::Partial(message)
        }
    }

    fn incomplete_message(&self, operation: &str, terminal_reason: Option<&str>) -> String {
        let mut message = format!(
            "{operation}{}：已应用 {} 项更改",
            if self.applied_changes == 0 {
                "失败"
            } else {
                "部分完成"
            },
            self.applied_changes
        );
        if self.failed_changes > 0 {
            message.push_str(&format!("；{} 项失败", self.failed_changes));
        }
        for detail in &self.failure_details {
            message.push('；');
            message.push_str(detail);
        }
        if let Some(reason) = terminal_reason {
            message.push('；');
            message.push_str(reason);
        }
        message
    }
}

use crate::tasks::{TaskManager, TaskOutcome};

use super::connections::ConnectionManager;

#[derive(Clone)]
pub struct TransferService {
    connections: ConnectionManager,
    tasks: TaskManager,
}

impl TransferService {
    pub(super) fn new(connections: ConnectionManager, tasks: TaskManager) -> Self {
        Self { connections, tasks }
    }
}

#[cfg(test)]
mod tests {
    use super::{TransferEffects, TransferFailure};
    use crate::tasks::TaskOutcome;

    #[test]
    fn interruptions_become_partial_after_a_durable_change() {
        let untouched = TransferEffects::default();
        assert!(matches!(
            untouched.interrupted("复制", TransferFailure::cancelled("任务已取消")),
            TaskOutcome::Cancelled(message) if message == "任务已取消"
        ));

        let mut changed = TransferEffects::default();
        changed.record_applied(2);
        assert!(matches!(
            changed.interrupted("恢复", TransferFailure::failed("连接已断开")),
            TaskOutcome::Partial(message)
                if message == "恢复部分完成：已应用 2 项更改；连接已断开"
        ));
    }

    #[test]
    fn failed_changes_are_failed_without_effects_and_partial_after_effects() {
        let mut failed = TransferEffects::default();
        failed.record_failure(2, "2 条语句执行失败");
        assert!(matches!(
            failed.finish("恢复", |_| "恢复完成".to_string()),
            TaskOutcome::Failed(message)
                if message == "恢复失败：已应用 0 项更改；2 项失败；2 条语句执行失败"
        ));

        let mut partial = TransferEffects::default();
        partial.record_applied(1);
        partial.record_failure(2, "2 行写入失败");
        assert!(matches!(
            partial.finish("复制", |_| "复制完成".to_string()),
            TaskOutcome::Partial(message)
                if message == "复制部分完成：已应用 1 项更改；2 项失败；2 行写入失败"
        ));

        let mut failed_then_cancelled = TransferEffects::default();
        failed_then_cancelled.record_failure(2, "2 条语句执行失败");
        assert!(matches!(
            failed_then_cancelled.interrupted(
                "恢复",
                TransferFailure::cancelled("任务已取消")
            ),
            TaskOutcome::Failed(message)
                if message == "恢复失败：已应用 0 项更改；2 项失败；2 条语句执行失败；任务已取消"
        ));
    }
}
