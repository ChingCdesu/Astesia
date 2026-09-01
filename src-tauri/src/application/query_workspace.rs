use std::ops::Range;

use crate::db::{DbType, SqlScript, StatementResult};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryTarget {
    pub connection_id: String,
    pub connection_name: String,
    pub database: String,
    pub db_type: DbType,
    pub session_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryExecutionScope {
    All,
    Current,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryDocument {
    text: String,
    selection: Range<usize>,
}

impl QueryDocument {
    pub fn new(text: String, selection: Range<usize>) -> Self {
        Self { text, selection }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QueryWorkspaceError {
    pub code: &'static str,
    pub message: String,
}

impl QueryWorkspaceError {
    fn target_required() -> Self {
        Self {
            code: "query_target_required",
            message: "请先在已连接的 Connection Profile 下选择数据库。".to_string(),
        }
    }

    fn unsupported_engine(db_type: DbType) -> Self {
        Self {
            code: "query_engine_unsupported",
            message: format!("{db_type:?} 不支持 SQL 查询；请使用对应的数据浏览器。"),
        }
    }

    fn invalid_selection() -> Self {
        Self {
            code: "query_selection_invalid",
            message: "编辑器选区已失效，请重新选择后再执行。".to_string(),
        }
    }

    fn empty() -> Self {
        Self {
            code: "query_empty",
            message: "没有可执行的 SQL 语句。".to_string(),
        }
    }

    fn parse(error: impl std::fmt::Display) -> Self {
        Self {
            code: "query_parse_failed",
            message: format!("SQL 解析失败：{error}"),
        }
    }

    fn execution(message: String) -> Self {
        Self {
            code: "query_execution_failed",
            message,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct QueryExecutionRequest {
    generation: u64,
    pub(crate) target: QueryTarget,
    pub(crate) statements: Vec<String>,
}

#[derive(Default)]
pub(crate) struct QueryWorkspaceState {
    target: Option<QueryTarget>,
    next_generation: u64,
    active_generation: Option<u64>,
    results: Vec<StatementResult>,
    active_result_index: usize,
    error: Option<QueryWorkspaceError>,
}

impl QueryWorkspaceState {
    pub(crate) fn target(&self) -> Option<&QueryTarget> {
        self.target.as_ref()
    }

    pub(crate) fn set_target(&mut self, target: Option<QueryTarget>) -> bool {
        if self.target == target {
            return false;
        }

        self.target = target;
        self.active_generation = None;
        self.results.clear();
        self.active_result_index = 0;
        self.error = None;
        true
    }

    pub(crate) fn is_running(&self) -> bool {
        self.active_generation.is_some()
    }

    pub(crate) fn results(&self) -> &[StatementResult] {
        &self.results
    }

    pub(crate) fn active_result_index(&self) -> usize {
        self.active_result_index
    }

    pub(crate) fn active_result(&self) -> Option<&StatementResult> {
        self.results.get(self.active_result_index)
    }

    pub(crate) fn error(&self) -> Option<&QueryWorkspaceError> {
        self.error.as_ref()
    }

    pub(crate) fn clear_results(&mut self) {
        self.active_generation = None;
        self.results.clear();
        self.active_result_index = 0;
        self.error = None;
    }

    pub(crate) fn select_result(&mut self, index: usize) -> bool {
        if index >= self.results.len() || self.active_result_index == index {
            return false;
        }
        self.active_result_index = index;
        true
    }

    pub(crate) fn begin_execution(
        &mut self,
        document: QueryDocument,
        scope: QueryExecutionScope,
    ) -> Result<QueryExecutionRequest, QueryWorkspaceError> {
        let preparation = self.prepare_execution(document, scope);
        let (target, statements) = match preparation {
            Ok(preparation) => preparation,
            Err(error) => {
                self.error = Some(error.clone());
                return Err(error);
            }
        };
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("query execution generation exhausted");
        self.active_generation = Some(self.next_generation);
        self.results.clear();
        self.active_result_index = 0;
        self.error = None;
        Ok(QueryExecutionRequest {
            generation: self.next_generation,
            target,
            statements,
        })
    }

    pub(crate) fn finish_execution(
        &mut self,
        request: &QueryExecutionRequest,
        result: Result<Vec<StatementResult>, String>,
    ) -> bool {
        if self.active_generation != Some(request.generation)
            || self.target.as_ref() != Some(&request.target)
        {
            return false;
        }

        self.active_generation = None;
        match result {
            Ok(results) => {
                self.active_result_index = results
                    .iter()
                    .position(|result| !result.success)
                    .unwrap_or(0);
                self.results = results;
                self.error = None;
            }
            Err(message) => {
                self.results.clear();
                self.active_result_index = 0;
                self.error = Some(QueryWorkspaceError::execution(message));
            }
        }
        true
    }

    fn prepare_execution(
        &self,
        document: QueryDocument,
        scope: QueryExecutionScope,
    ) -> Result<(QueryTarget, Vec<String>), QueryWorkspaceError> {
        let target = self
            .target
            .clone()
            .ok_or_else(QueryWorkspaceError::target_required)?;
        if !target.db_type.capabilities().sql {
            return Err(QueryWorkspaceError::unsupported_engine(target.db_type));
        }
        let statements = select_statements(target.db_type, document, scope)?;
        Ok((target, statements))
    }
}

fn select_statements(
    db_type: DbType,
    document: QueryDocument,
    scope: QueryExecutionScope,
) -> Result<Vec<String>, QueryWorkspaceError> {
    validate_range(&document.text, &document.selection)?;
    if !document.selection.is_empty() {
        return parse_statements(db_type, &document.text[document.selection]);
    }

    let script = SqlScript::parse(db_type, &document.text).map_err(QueryWorkspaceError::parse)?;
    match scope {
        QueryExecutionScope::All => non_empty(script.into_statements()),
        QueryExecutionScope::Current => script
            .statement_at(document.selection.start)
            .map(|statement| vec![statement.to_string()])
            .ok_or_else(QueryWorkspaceError::empty),
    }
}

fn parse_statements(db_type: DbType, source: &str) -> Result<Vec<String>, QueryWorkspaceError> {
    let statements = SqlScript::parse(db_type, source)
        .map_err(QueryWorkspaceError::parse)?
        .into_statements();
    non_empty(statements)
}

fn non_empty(statements: Vec<String>) -> Result<Vec<String>, QueryWorkspaceError> {
    if statements.is_empty() {
        Err(QueryWorkspaceError::empty())
    } else {
        Ok(statements)
    }
}

fn validate_range(source: &str, range: &Range<usize>) -> Result<(), QueryWorkspaceError> {
    if range.start > range.end
        || range.end > source.len()
        || !source.is_char_boundary(range.start)
        || !source.is_char_boundary(range.end)
    {
        return Err(QueryWorkspaceError::invalid_selection());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn target(id: &str, generation: u64, db_type: DbType) -> QueryTarget {
        QueryTarget {
            connection_id: id.to_string(),
            connection_name: id.to_string(),
            database: "app".to_string(),
            db_type,
            session_generation: generation,
        }
    }

    fn result(sql: &str, success: bool) -> StatementResult {
        StatementResult {
            sql: sql.to_string(),
            success,
            error: (!success).then(|| "failed".to_string()),
            columns: Vec::new(),
            rows: vec![vec![json!(1)]],
            affected_rows: 0,
            execution_time_ms: 1,
        }
    }

    #[test]
    fn selection_takes_precedence_and_current_uses_the_cursor_statement() {
        let mut state = QueryWorkspaceState::default();
        state.set_target(Some(target("primary", 1, DbType::PostgreSQL)));
        let sql = "SELECT 1;\n-- gap\nSELECT '二';".to_string();

        let selected = state
            .begin_execution(
                QueryDocument::new(sql.clone(), 0..sql.find(';').unwrap()),
                QueryExecutionScope::Current,
            )
            .unwrap();
        assert_eq!(selected.statements, vec!["SELECT 1"]);

        let cursor = sql.find('二').unwrap();
        let current = state
            .begin_execution(
                QueryDocument::new(sql, cursor..cursor),
                QueryExecutionScope::Current,
            )
            .unwrap();
        assert_eq!(current.statements, vec!["SELECT '二'"]);
    }

    #[test]
    fn all_executes_every_dialect_aware_statement() {
        let mut state = QueryWorkspaceState::default();
        state.set_target(Some(target("primary", 1, DbType::PostgreSQL)));

        let request = state
            .begin_execution(
                QueryDocument::new("SELECT ';'; SELECT 2;".to_string(), 0..0),
                QueryExecutionScope::All,
            )
            .unwrap();

        assert_eq!(request.statements, vec!["SELECT ';'", "SELECT 2"]);
    }

    #[test]
    fn target_changes_discard_in_flight_results() {
        let mut state = QueryWorkspaceState::default();
        state.set_target(Some(target("primary", 1, DbType::MySQL)));
        let request = state
            .begin_execution(
                QueryDocument::new("SELECT 1".to_string(), 0..0),
                QueryExecutionScope::All,
            )
            .unwrap();

        state.set_target(Some(target("primary", 2, DbType::MySQL)));
        assert!(!state.finish_execution(&request, Ok(vec![result("SELECT 1", true)])));
        assert!(state.results().is_empty());
    }

    #[test]
    fn completion_selects_the_first_failed_statement() {
        let mut state = QueryWorkspaceState::default();
        state.set_target(Some(target("primary", 1, DbType::SQLite)));
        let request = state
            .begin_execution(
                QueryDocument::new("SELECT 1; SELECT 2".to_string(), 0..0),
                QueryExecutionScope::All,
            )
            .unwrap();

        assert!(state.finish_execution(
            &request,
            Ok(vec![result("SELECT 1", true), result("SELECT 2", false)])
        ));
        assert_eq!(state.active_result_index(), 1);
        assert_eq!(state.active_result().unwrap().sql, "SELECT 2");
    }

    #[test]
    fn invalid_selection_and_non_sql_targets_fail_before_execution() {
        let mut state = QueryWorkspaceState::default();
        state.set_target(Some(target("primary", 1, DbType::PostgreSQL)));
        let error = state
            .begin_execution(
                QueryDocument::new("SELECT '二'".to_string(), 9..10),
                QueryExecutionScope::All,
            )
            .unwrap_err();
        assert_eq!(error.code, "query_selection_invalid");

        state.set_target(Some(target("redis", 1, DbType::Redis)));
        let error = state
            .begin_execution(
                QueryDocument::new("GET key".to_string(), 0..0),
                QueryExecutionScope::All,
            )
            .unwrap_err();
        assert_eq!(error.code, "query_engine_unsupported");
    }

    #[test]
    fn execution_failures_are_visible_and_a_new_run_clears_them() {
        let mut state = QueryWorkspaceState::default();
        state.set_target(Some(target("primary", 1, DbType::ClickHouse)));
        let request = state
            .begin_execution(
                QueryDocument::new("SELECT 1".to_string(), 0..0),
                QueryExecutionScope::All,
            )
            .unwrap();
        state.finish_execution(&request, Err("driver stopped".to_string()));
        assert_eq!(state.error().unwrap().message, "driver stopped");

        state
            .begin_execution(
                QueryDocument::new("SELECT 2".to_string(), 0..0),
                QueryExecutionScope::All,
            )
            .unwrap();
        assert!(state.error().is_none());
        assert!(state.is_running());
    }
}
