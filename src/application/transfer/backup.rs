mod execute;
mod plan;
mod render;

use crate::{application::QueryTarget, db::TableRef, tasks::NewTask};

use self::{execute::execute_backup, plan::BackupPlan};
use super::{BackupContent, TransferService};

#[derive(Debug)]
pub struct BackupOptions {
    pub tables: Option<Vec<TableRef>>,
    pub content: BackupContent,
    pub drop_tables: DropTableMode,
    pub output_path: String,
}

impl BackupOptions {
    fn validate(&self) -> Result<(), String> {
        if self.drop_tables != DropTableMode::None && !self.content.includes_structure() {
            return Err("仅备份数据时不能生成 DROP TABLE 语句".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropTableMode {
    None,
    Drop,
    DropIfExists,
}

impl TransferService {
    pub async fn start_backup(
        &self,
        target: QueryTarget,
        options: BackupOptions,
    ) -> Result<String, String> {
        options.validate()?;

        let driver = self.driver_for_target(&target).await?;
        let plan = BackupPlan::discover(&driver, target.database, options).await?;
        let task_name = format!("备份 {}", plan.database);

        let task_id = self
            .tasks
            .spawn(
                NewTask {
                    name: task_name,
                    initial_message: "开始备份...".to_string(),
                },
                move |task| execute_backup(plan, driver, task),
            )
            .await;

        Ok(task_id)
    }
}

#[cfg(test)]
mod tests {
    use super::{BackupContent, BackupOptions, DropTableMode};

    #[test]
    fn rejects_drop_statements_from_data_only_backups() {
        let options = BackupOptions {
            tables: None,
            content: BackupContent::Data,
            drop_tables: DropTableMode::DropIfExists,
            output_path: "backup.sql".to_string(),
        };

        assert_eq!(
            options.validate(),
            Err("仅备份数据时不能生成 DROP TABLE 语句".to_string())
        );
    }
}
