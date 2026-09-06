use super::QueryTarget;
#[cfg(test)]
mod engine_tests;
mod stream;
#[cfg(test)]
mod tests;
use crate::db::{DatabaseTransaction, QueryResult, StatementResult};
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Clone)]
pub(crate) struct GridTransaction {
    target: QueryTarget,
    commands: mpsc::Sender<Command>,
    applied: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

enum Operation {
    Stream {
        sql: String,
        sink: stream::ChannelSink,
    },
    Batch {
        statements: Vec<String>,
        record: bool,
    },
    Finish {
        commit: bool,
    },
}

struct Command {
    operation: Operation,
    reply: oneshot::Sender<Result<Vec<StatementResult>, String>>,
}

impl GridTransaction {
    pub(super) fn start(
        target: QueryTarget,
        mut transaction: Box<dyn DatabaseTransaction>,
        mut retirement: watch::Receiver<bool>,
    ) -> Self {
        let (commands, mut incoming) = mpsc::channel::<Command>(8);
        let applied = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let pending = applied.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = retirement.wait_for(|retired| *retired) => {},
                _ = async move {
                    while let Some(command) = incoming.recv().await {
                        if command.reply.is_closed() { continue; }
                        match command.operation {
                            Operation::Stream { sql, mut sink } => {
                                let result = stream::read(&mut *transaction, sql, &mut sink).await;
                                let unusable = result.is_err();
                                let _ = command.reply.send(result.map(|result| vec![result]).map_err(|error| error.to_string()));
                                if unusable { break; }
                            }
                            Operation::Batch { statements, record } => {
                                let recovery = record.then(|| statements.clone());
                                let result = transaction.apply_batch(statements).await.map_err(|error| error.to_string());
                                if result.as_ref().is_ok_and(|results| results.iter().all(|result| result.success)) {
                                    if let Some(recovery) = recovery { pending.lock().expect("transaction recovery lock").extend(recovery); }
                                }
                                let unusable = result.is_err();
                                let _ = command.reply.send(result);
                                if unusable { break; }
                            }
                            Operation::Finish { commit } => {
                                let result = if commit { transaction.commit().await } else { transaction.rollback().await };
                                if result.is_ok() { pending.lock().expect("transaction recovery lock").clear(); }
                                let _ = command.reply.send(result.map(|_| Vec::new()).map_err(|error| {
                                    format!("Transaction finalization failed; refresh to verify the database outcome: {error}")
                                }));
                                break;
                            }
                        }
                    }
                } => {},
            }
        });
        Self {
            target,
            commands,
            applied,
        }
    }

    pub(crate) fn target(&self) -> &QueryTarget {
        &self.target
    }
    pub(crate) fn is_closed(&self) -> bool {
        self.commands.is_closed()
    }

    pub(crate) fn has_pending_changes(&self) -> bool {
        !self
            .applied
            .lock()
            .expect("transaction recovery lock")
            .is_empty()
    }

    pub(crate) fn recovery_sql(&self) -> String {
        self.applied
            .lock()
            .expect("transaction recovery lock")
            .iter()
            .map(|statement| format!("{statement};\n"))
            .collect()
    }

    async fn request(&self, operation: Operation) -> Result<Vec<StatementResult>, String> {
        let (reply, receive) = oneshot::channel();
        self.commands
            .send(Command { operation, reply })
            .await
            .map_err(|_| "Transaction session is closed".to_string())?;
        receive.await.map_err(|_| "Transaction session ended before confirmation; refresh to verify the database outcome".to_string())?
    }

    pub(crate) async fn apply(
        &self,
        statements: Vec<String>,
    ) -> Result<Vec<StatementResult>, String> {
        self.request(Operation::Batch {
            statements,
            record: true,
        })
        .await
    }

    pub(crate) async fn query(&self, sql: String) -> Result<QueryResult, String> {
        let result = self
            .request(Operation::Batch {
                statements: vec![sql],
                record: false,
            })
            .await?
            .into_iter()
            .next()
            .ok_or("Missing query result")?;
        if !result.success {
            return Err(result
                .error
                .unwrap_or_else(|| "Transaction query failed".to_string()));
        }
        Ok(QueryResult {
            columns: result.columns,
            rows: result.rows,
            affected_rows: result.affected_rows,
            execution_time_ms: result.execution_time_ms,
        })
    }

    pub(crate) async fn finish(&self, commit: bool) -> Result<(), String> {
        self.request(Operation::Finish { commit }).await.map(|_| ())
    }
}
