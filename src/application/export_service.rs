mod writer;

use crate::db::QueryRowSink;
use serde_json::Value;

use crate::tasks::{NewTask, TaskContext, TaskManager, TaskOutcome};

use super::{GridTransaction, QueryService, QueryTarget};

/// Where the rows to export come from.
///
/// `Sql` keeps large query results inside the export workflow; `Rows` preserves
/// an already-materialized selection exactly as supplied by the caller.
#[derive(Debug)]
pub enum ExportSource {
    Sql {
        sql: String,
    },
    Rows {
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
    },
}

#[derive(Debug)]
pub struct CsvOptions {
    pub delimiter: String,
    pub include_header: bool,
    pub quote_all: bool,
    pub null_value: String,
    pub crlf: bool,
    pub bom: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonLayout {
    Objects,
    Arrays,
}

#[derive(Debug)]
pub struct JsonOptions {
    pub layout: JsonLayout,
    pub pretty: bool,
}

#[derive(Debug)]
pub struct XlsxOptions {
    pub include_header: bool,
    pub sheet_name: String,
}

#[derive(Debug)]
pub enum ExportFormat {
    Csv(CsvOptions),
    Json(JsonOptions),
    Xlsx(XlsxOptions),
}

#[derive(Clone)]
pub struct ExportService {
    queries: QueryService,
    tasks: TaskManager,
}

impl ExportService {
    pub(super) fn new(queries: QueryService, tasks: TaskManager) -> Self {
        Self { queries, tasks }
    }

    pub(crate) async fn start_export(
        &self,
        target: QueryTarget,
        source: ExportSource,
        format: ExportFormat,
        output_path: String,
    ) -> Result<String, String> {
        self.start_export_from(target, source, format, output_path, None)
            .await
    }

    pub(crate) async fn start_transaction_export(
        &self,
        transaction: GridTransaction,
        source: ExportSource,
        format: ExportFormat,
        output_path: String,
    ) -> Result<String, String> {
        self.start_export_from(
            transaction.target().clone(),
            source,
            format,
            output_path,
            Some(transaction),
        )
        .await
    }

    async fn start_export_from(
        &self,
        target: QueryTarget,
        source: ExportSource,
        format: ExportFormat,
        output_path: String,
        transaction: Option<GridTransaction>,
    ) -> Result<String, String> {
        let service = self.clone();
        let name = std::path::Path::new(&output_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("export")
            .to_string();
        Ok(self
            .tasks
            .spawn(
                NewTask {
                    name: format!("Export {name}"),
                    initial_message: "Preparing export...".to_string(),
                },
                move |task| async move {
                    task.progress(0.1, "Writing export rows...").await;
                    let result = service
                        .export_to_file(
                            transaction
                                .as_ref()
                                .map(ExportTarget::Transaction)
                                .unwrap_or(ExportTarget::Session(&target)),
                            source,
                            format,
                            output_path,
                            Some(task.clone()),
                        )
                        .await;
                    match result {
                        Ok(count) => TaskOutcome::Completed(format!("Exported {count} row(s)")),
                        Err(_) if task.is_cancelled() => TaskOutcome::Cancelled(
                            "Export cancelled before the output file was written".to_string(),
                        ),
                        Err(error) => TaskOutcome::Failed(error),
                    }
                },
            )
            .await)
    }

    pub async fn export(
        &self,
        connection_id: &str,
        database: &str,
        source: ExportSource,
        format: ExportFormat,
        output_path: String,
    ) -> Result<usize, String> {
        self.export_to_file(
            ExportTarget::Current {
                connection_id,
                database,
            },
            source,
            format,
            output_path,
            None,
        )
        .await
    }

    async fn export_to_file(
        &self,
        target: ExportTarget<'_>,
        source: ExportSource,
        format: ExportFormat,
        output_path: String,
        task: Option<TaskContext>,
    ) -> Result<usize, String> {
        if task.as_ref().is_some_and(TaskContext::is_cancelled) {
            return Err("Export cancelled".to_string());
        }
        let (sender, receiver) = tokio::sync::mpsc::channel(32);
        let writer_task = task.clone();
        let writer = tokio::task::spawn_blocking(move || {
            writer::write_export(receiver, format, output_path, writer_task)
        });
        let mut sink = ExportSink {
            sender,
            task,
            headers_sent: false,
        };
        let result = match source {
            ExportSource::Rows { columns, rows } => {
                sink.headers(columns).await;
                for row in rows {
                    if !sink.wants_rows() {
                        break;
                    }
                    sink.row(row).await;
                }
                Ok(())
            }
            ExportSource::Sql { sql } => match target {
                ExportTarget::Session(target) => self
                    .queries
                    .execute_export_query(target, &sql, &mut sink)
                    .await
                    .map(|_| ()),
                ExportTarget::Current {
                    connection_id,
                    database,
                } => self
                    .queries
                    .stream_export(connection_id, database, &sql, &mut sink)
                    .await
                    .map(|_| ()),
                ExportTarget::Transaction(transaction) => {
                    transaction.stream_rows(sql, &mut sink).await
                }
            },
        };
        let cancelled = sink.task.as_ref().is_some_and(TaskContext::is_cancelled);
        if result.is_ok() && !cancelled {
            if !sink.headers_sent {
                sink.headers(Vec::new()).await;
            }
            let _ = sink.sender.send(ExportEvent::Finish).await;
        }
        drop(sink);
        let written = writer
            .await
            .map_err(|error| format!("导出任务失败: {error}"))?;
        result?;
        if cancelled {
            return Err("Export cancelled".to_string());
        }
        written
    }
}

enum ExportTarget<'a> {
    Transaction(&'a GridTransaction),
    Current {
        connection_id: &'a str,
        database: &'a str,
    },
    Session(&'a QueryTarget),
}

pub(super) enum ExportEvent {
    Columns(Vec<String>),
    Row(Vec<Value>),
    Finish,
}

struct ExportSink {
    sender: tokio::sync::mpsc::Sender<ExportEvent>,
    task: Option<TaskContext>,
    headers_sent: bool,
}

impl ExportSink {
    async fn headers(&mut self, columns: Vec<String>) {
        if !self.headers_sent {
            self.headers_sent = true;
            let _ = self.sender.send(ExportEvent::Columns(columns)).await;
        }
    }
}

#[async_trait::async_trait]
impl QueryRowSink for ExportSink {
    fn wants_rows(&self) -> bool {
        !self.sender.is_closed() && !self.task.as_ref().is_some_and(TaskContext::is_cancelled)
    }

    async fn columns(&mut self, columns: &[crate::db::ColumnInfo]) {
        self.headers(columns.iter().map(|column| column.name.clone()).collect())
            .await;
    }

    async fn row(&mut self, row: Vec<Value>) {
        let _ = self.sender.send(ExportEvent::Row(row)).await;
    }
}
