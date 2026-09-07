use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use super::{Command, GridTransaction, Operation};
use crate::db::{ColumnInfo, DatabaseTransaction, DbType, QueryRowSink, StatementResult};
use tokio::sync::{mpsc, oneshot};

enum Event {
    Columns(Vec<ColumnInfo>),
    Row(Vec<serde_json::Value>),
}

pub(super) struct ChannelSink {
    sender: mpsc::Sender<Event>,
    accepting: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl QueryRowSink for ChannelSink {
    fn wants_rows(&self) -> bool {
        !self.sender.is_closed() && self.accepting.load(Ordering::Relaxed)
    }

    async fn columns(&mut self, columns: &[ColumnInfo]) {
        let _ = self.sender.send(Event::Columns(columns.to_vec())).await;
    }

    async fn row(&mut self, row: Vec<serde_json::Value>) {
        if self.wants_rows() {
            let _ = self.sender.send(Event::Row(row)).await;
        }
    }
}

impl GridTransaction {
    pub(crate) async fn stream_rows(
        &self,
        sql: String,
        sink: &mut dyn QueryRowSink,
    ) -> Result<(), String> {
        let (sender, mut rows) = mpsc::channel(32);
        let accepting = Arc::new(AtomicBool::new(true));
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command {
                operation: Operation::Stream {
                    sql,
                    sink: ChannelSink {
                        sender,
                        accepting: accepting.clone(),
                    },
                },
                reply,
            })
            .await
            .map_err(|_| "Transaction session is closed".to_string())?;
        while let Some(event) = rows.recv().await {
            accepting.store(sink.wants_rows(), Ordering::Relaxed);
            match event {
                Event::Columns(columns) => sink.columns(&columns).await,
                Event::Row(row) if sink.wants_rows() => sink.row(row).await,
                Event::Row(_) => {}
            }
        }
        let results = result
            .await
            .map_err(|_| "Transaction stream ended before confirmation".to_string())??;
        let result = results
            .into_iter()
            .next()
            .ok_or("Missing transaction stream result")?;
        if result.success {
            Ok(())
        } else {
            Err(result
                .error
                .unwrap_or_else(|| "Transaction stream failed".to_string()))
        }
    }
}

pub(super) async fn read(
    transaction: &mut dyn DatabaseTransaction,
    sql: String,
    sink: &mut dyn QueryRowSink,
) -> anyhow::Result<StatementResult> {
    let sql_server = transaction.db_type() == DbType::SQLServer;
    transaction
        .execute(if sql_server {
            "SAVE TRANSACTION astesia_grid_export"
        } else {
            "SAVEPOINT astesia_grid_export"
        })
        .await?;
    let started = std::time::Instant::now();
    let result = transaction.execute_stream(&sql, sink).await;
    if result.is_err() {
        transaction
            .execute(if sql_server {
                "ROLLBACK TRANSACTION astesia_grid_export"
            } else {
                "ROLLBACK TO SAVEPOINT astesia_grid_export"
            })
            .await?;
    }
    if !sql_server {
        transaction
            .execute("RELEASE SAVEPOINT astesia_grid_export")
            .await?;
    }
    Ok(match result {
        Ok(result) => StatementResult::from_query_result(sql, result),
        Err(error) => StatementResult::from_error(sql, error, started.elapsed().as_millis() as u64),
    })
}
