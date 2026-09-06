use async_trait::async_trait;
use serde_json::Value;

use super::{ColumnInfo, QueryResult};

/// Row retention is separate from execution: drivers drain every response even
/// after a consumer stops accepting rows, so a limit cannot interrupt mutations.
#[async_trait]
pub trait QueryRowSink: Send {
    fn wants_rows(&self) -> bool {
        true
    }

    async fn columns(&mut self, _columns: &[ColumnInfo]) {}

    async fn row(&mut self, row: Vec<Value>);
}

pub(crate) struct QueryRowCollector {
    rows: Vec<Vec<Value>>,
    limit: Option<usize>,
}

impl QueryRowCollector {
    pub(crate) fn new(limit: Option<usize>) -> Self {
        Self {
            rows: Vec::new(),
            limit,
        }
    }

    pub(crate) fn finish(self, mut result: QueryResult) -> QueryResult {
        result.rows = self.rows;
        result
    }
}

#[async_trait]
impl QueryRowSink for QueryRowCollector {
    fn wants_rows(&self) -> bool {
        self.limit.is_none_or(|limit| self.rows.len() < limit)
    }

    async fn row(&mut self, row: Vec<Value>) {
        if self.wants_rows() {
            self.rows.push(row);
        }
    }
}
