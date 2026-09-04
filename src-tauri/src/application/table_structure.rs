use crate::db::{ColumnInfo, ConstraintInfo, ForeignKeyInfo, IndexInfo, TableRef};

use super::QueryTarget;

#[derive(Clone, Debug)]
pub(crate) struct TableStructureSnapshot {
    pub(crate) columns: Vec<ColumnInfo>,
    pub(crate) indexes: Vec<IndexInfo>,
    pub(crate) constraints: Option<Vec<ConstraintInfo>>,
    pub(crate) foreign_keys: Option<Vec<ForeignKeyInfo>>,
}

#[derive(Clone, Debug)]
pub(crate) enum TableStructureLoadError {
    Connection(String),
    Unsupported(String),
    Columns(String),
    Indexes(String),
    Constraints(String),
    ForeignKeys(String),
    BackgroundTask(String),
}

impl TableStructureLoadError {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Connection(message)
            | Self::Unsupported(message)
            | Self::Columns(message)
            | Self::Indexes(message)
            | Self::Constraints(message)
            | Self::ForeignKeys(message)
            | Self::BackgroundTask(message) => message,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TableStructureRequest {
    generation: u64,
}

#[derive(Debug)]
enum TableStructurePhase {
    Idle,
    Loading { generation: u64 },
    Ready(TableStructureSnapshot),
    Failed(TableStructureLoadError),
    Unavailable(String),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TableStructureStatus<'a> {
    Idle,
    Loading,
    Ready(&'a TableStructureSnapshot),
    Failed(&'a TableStructureLoadError),
    Unavailable(&'a str),
}

#[derive(Debug)]
pub(crate) struct TableStructureState {
    target: QueryTarget,
    table: TableRef,
    next_generation: u64,
    phase: TableStructurePhase,
}

impl TableStructureState {
    pub(crate) fn new(target: QueryTarget, table: TableRef) -> Self {
        Self {
            target,
            table,
            next_generation: 0,
            phase: TableStructurePhase::Idle,
        }
    }

    pub(crate) fn target(&self) -> &QueryTarget {
        &self.target
    }

    pub(crate) fn table(&self) -> &TableRef {
        &self.table
    }

    pub(crate) fn status(&self) -> TableStructureStatus<'_> {
        match &self.phase {
            TableStructurePhase::Idle => TableStructureStatus::Idle,
            TableStructurePhase::Loading { .. } => TableStructureStatus::Loading,
            TableStructurePhase::Ready(snapshot) => TableStructureStatus::Ready(snapshot),
            TableStructurePhase::Failed(error) => TableStructureStatus::Failed(error),
            TableStructurePhase::Unavailable(reason) => TableStructureStatus::Unavailable(reason),
        }
    }

    pub(crate) fn begin_load(&mut self) -> Option<TableStructureRequest> {
        if matches!(self.phase, TableStructurePhase::Loading { .. }) {
            return None;
        }
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("table structure request generation exhausted");
        let request = TableStructureRequest {
            generation: self.next_generation,
        };
        self.phase = TableStructurePhase::Loading {
            generation: request.generation,
        };
        Some(request)
    }

    pub(crate) fn finish_load(
        &mut self,
        request: TableStructureRequest,
        result: Result<TableStructureSnapshot, TableStructureLoadError>,
    ) -> bool {
        if !matches!(
            self.phase,
            TableStructurePhase::Loading { generation }
                if generation == request.generation
        ) {
            return false;
        }
        self.phase = match result {
            Ok(snapshot) => TableStructurePhase::Ready(snapshot),
            Err(error) => TableStructurePhase::Failed(error),
        };
        true
    }

    pub(crate) fn invalidate_session(
        &mut self,
        connection_id: &str,
        session_generation: u64,
        reason: impl Into<String>,
    ) -> bool {
        if self.target.connection_id != connection_id
            || self.target.session_generation != session_generation
            || matches!(self.phase, TableStructurePhase::Unavailable(_))
        {
            return false;
        }
        self.phase = TableStructurePhase::Unavailable(reason.into());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbType;

    fn target(session_generation: u64) -> QueryTarget {
        QueryTarget {
            connection_id: "primary".to_string(),
            connection_name: "Primary".to_string(),
            database: "app".to_string(),
            db_type: DbType::PostgreSQL,
            session_generation,
        }
    }

    fn snapshot() -> TableStructureSnapshot {
        TableStructureSnapshot {
            columns: vec![ColumnInfo {
                name: "id".to_string(),
                data_type: "bigint".to_string(),
                nullable: false,
                is_primary_key: true,
                default_value: None,
                comment: None,
            }],
            indexes: vec![IndexInfo {
                name: "users_pkey".to_string(),
                columns: vec!["id".to_string()],
                is_unique: true,
                is_primary: true,
            }],
            constraints: Some(Vec::new()),
            foreign_keys: Some(Vec::new()),
        }
    }

    #[test]
    fn stale_completions_cannot_replace_a_newer_table_structure_load() {
        let mut state = TableStructureState::new(target(4), TableRef::unqualified("users"));
        let stale = state.begin_load().unwrap();
        assert!(state.finish_load(
            stale,
            Err(TableStructureLoadError::Columns("temporary".to_string()))
        ));
        let current = state.begin_load().unwrap();

        assert!(!state.finish_load(stale, Ok(snapshot())));
        assert!(state.finish_load(current, Ok(snapshot())));
        assert!(matches!(
            state.status(),
            TableStructureStatus::Ready(snapshot)
                if snapshot.columns.len() == 1 && snapshot.indexes.len() == 1
        ));
    }

    #[test]
    fn session_invalidation_rejects_an_in_flight_completion() {
        let mut state = TableStructureState::new(target(7), TableRef::unqualified("users"));
        let request = state.begin_load().unwrap();

        assert!(state.invalidate_session("primary", 7, "Session changed"));
        assert!(!state.finish_load(request, Ok(snapshot())));
        assert!(matches!(
            state.status(),
            TableStructureStatus::Unavailable("Session changed")
        ));
    }
}
