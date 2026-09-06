use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
    fmt,
};

use crate::db::{
    ColumnInfo, DatabaseDriver, DbType, QueryResult, RowMutationMode, SqlDialect, SqlScript,
    StatementResult, TableRef,
};

use super::connections::ConnectionManager;
use super::{
    GridLoadRequest, GridPage, GridSaveFailure, GridSavePlan, GridSaveRequest, GridSessionError,
    GridSort, GridSortDirection,
};

#[derive(Clone)]
pub(crate) struct GridService {
    manager: ConnectionManager,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GridLoadError {
    Connection(String),
    SessionChanged { expected: u64, actual: u64 },
    EngineChanged { expected: DbType, actual: DbType },
    Unsupported(DbType),
    Columns(String),
    Query(String),
    InvalidPage(GridSessionError),
}

impl fmt::Display for GridLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connection(message) => formatter.write_str(message),
            Self::SessionChanged { expected, actual } => write!(
                formatter,
                "Connection session changed before the grid loaded (expected {expected}, found {actual})"
            ),
            Self::EngineChanged { expected, actual } => write!(
                formatter,
                "Connection engine changed before the grid loaded (expected {expected:?}, found {actual:?})"
            ),
            Self::Unsupported(db_type) => {
                write!(formatter, "Data grids are not supported for {db_type:?}")
            }
            Self::Columns(message) => write!(formatter, "Could not load table columns: {message}"),
            Self::Query(message) => write!(formatter, "Could not load table rows: {message}"),
            Self::InvalidPage(error) => write!(formatter, "Invalid grid page: {error:?}"),
        }
    }
}

impl Error for GridLoadError {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct GridSaveOutcome {
    pub(crate) statements_executed: usize,
    pub(crate) changes_applied: usize,
    pub(crate) affected_rows: u64,
}

impl GridService {
    pub(super) fn new(manager: ConnectionManager) -> Self {
        Self { manager }
    }

    pub(crate) async fn load(&self, request: &GridLoadRequest) -> Result<GridPage, GridLoadError> {
        self.load_in(request, None).await
    }

    pub(crate) async fn begin_transaction(
        &self,
        target: super::QueryTarget,
        isolation: crate::db::TransactionIsolation,
    ) -> Result<super::GridTransaction, String> {
        let (handle, generation) = self.manager.driver_session(&target.connection_id).await?;
        if generation != target.session_generation {
            return Err("Connection session changed".to_string());
        }
        let driver = handle.lock_active().await?;
        if driver.db_type() != target.db_type {
            return Err("Connection engine changed".to_string());
        }
        let transaction = driver
            .begin_transaction(&target.database, isolation)
            .await
            .map_err(|error| error.to_string())?;
        Ok(super::GridTransaction::start(
            target,
            transaction,
            handle.retirement(),
        ))
    }

    pub(crate) async fn load_in(
        &self,
        request: &GridLoadRequest,
        transaction: Option<&super::GridTransaction>,
    ) -> Result<GridPage, GridLoadError> {
        let target = request.target();
        if transaction.is_some_and(|transaction| transaction.target() != target) {
            return Err(GridLoadError::Connection(
                "Transaction belongs to another database session".to_string(),
            ));
        }
        let (handle, actual_generation) = self
            .manager
            .driver_session(&target.connection_id)
            .await
            .map_err(GridLoadError::Connection)?;
        if actual_generation != target.session_generation {
            return Err(GridLoadError::SessionChanged {
                expected: target.session_generation,
                actual: actual_generation,
            });
        }
        let driver = handle
            .lock_active()
            .await
            .map_err(GridLoadError::Connection)?;
        let actual_db_type = driver.db_type();
        if actual_db_type != target.db_type {
            return Err(GridLoadError::EngineChanged {
                expected: target.db_type,
                actual: actual_db_type,
            });
        }
        if !actual_db_type.capabilities().sql {
            return Err(GridLoadError::Unsupported(actual_db_type));
        }

        let columns = driver
            .get_columns(&target.database, request.table())
            .await
            .map_err(|error| GridLoadError::Columns(error.to_string()))?;
        let enum_values = grid_enum_values(&*driver, actual_db_type, &target.database, &columns)
            .await
            .map_err(GridLoadError::Columns)?;
        let page_sql = grid_page_sql(
            actual_db_type,
            request.table(),
            request.query().page,
            request.query().page_size,
            request.query().filter.as_deref(),
            &request.query().sort,
            &columns,
        )
        .map_err(GridLoadError::Query)?;
        let count_sql = grid_count_sql(
            actual_db_type,
            request.table(),
            request.query().filter.as_deref(),
        )
        .map_err(GridLoadError::Query)?;
        let mut result = if let Some(transaction) = transaction {
            transaction
                .query(page_sql)
                .await
                .map_err(GridLoadError::Query)?
        } else {
            driver
                .execute_query(&target.database, &page_sql)
                .await
                .map_err(|error| GridLoadError::Query(error.to_string()))?
        };
        let count_result = if let Some(transaction) = transaction {
            transaction.query(count_sql).await.ok()
        } else {
            driver
                .execute_query(&target.database, &count_sql)
                .await
                .ok()
        };
        let total_rows = count_result.and_then(|result| total_rows(&result));
        normalize_grid_values(actual_db_type, &columns, &mut result)
            .map_err(GridLoadError::Query)?;
        merge_column_metadata(&mut result, columns);
        GridPage::new(result.columns, result.rows, total_rows)
            .map(|page| page.with_enum_values(enum_values))
            .map_err(GridLoadError::InvalidPage)
    }

    pub(crate) async fn save(
        &self,
        request: &GridSaveRequest,
    ) -> Result<GridSaveOutcome, GridSaveFailure> {
        self.save_in(request, None).await
    }

    pub(crate) async fn save_with_isolation(
        &self,
        request: &GridSaveRequest,
        isolation: crate::db::TransactionIsolation,
    ) -> Result<GridSaveOutcome, GridSaveFailure> {
        let count = save_statement_count(request.plan());
        let transaction = self
            .begin_transaction(request.plan().target.clone(), isolation)
            .await
            .map_err(|error| GridSaveFailure::before_execution(count, error))?;
        match self.save_in(request, Some(&transaction)).await {
            Ok(outcome) => {
                transaction.finish(true).await.map_err(|error| {
                    let mut failure = GridSaveFailure::before_execution(count, error);
                    failure.recovery_sql = Some(transaction.recovery_sql());
                    failure
                })?;
                Ok(outcome)
            }
            Err(mut failure) => {
                if let Err(error) = transaction.finish(false).await {
                    failure.message = format!("{}; {error}", failure.message);
                }
                Err(failure)
            }
        }
    }

    pub(crate) async fn save_in(
        &self,
        request: &GridSaveRequest,
        transaction: Option<&super::GridTransaction>,
    ) -> Result<GridSaveOutcome, GridSaveFailure> {
        let plan = request.plan();
        let total_statements = save_statement_count(plan);
        let target = &plan.target;
        if transaction.is_some_and(|transaction| transaction.target() != target) {
            return Err(GridSaveFailure::before_execution(
                total_statements,
                "Transaction belongs to another database session".to_string(),
            ));
        }
        let (handle, actual_generation) = self
            .manager
            .driver_session(&target.connection_id)
            .await
            .map_err(|message| GridSaveFailure::before_execution(total_statements, message))?;
        if actual_generation != target.session_generation {
            return Err(GridSaveFailure::before_execution(
                total_statements,
                format!(
                    "Connection session changed before the grid saved (expected {}, found {})",
                    target.session_generation, actual_generation
                ),
            ));
        }
        let driver = handle
            .lock_active()
            .await
            .map_err(|message| GridSaveFailure::before_execution(total_statements, message))?;
        let actual_db_type = driver.db_type();
        if actual_db_type != target.db_type {
            return Err(GridSaveFailure::before_execution(
                total_statements,
                format!(
                    "Connection engine changed before the grid saved (expected {:?}, found {actual_db_type:?})",
                    target.db_type
                ),
            ));
        }
        let capabilities = actual_db_type.capabilities();
        if capabilities.data_browser_read_only
            || capabilities.row_mutation != RowMutationMode::StructuredSql
        {
            return Err(GridSaveFailure::before_execution(
                total_statements,
                format!("Editable data grids are not supported for {actual_db_type:?}"),
            ));
        }

        let statements = save_statements(plan)
            .map_err(|message| GridSaveFailure::before_execution(total_statements, message))?;
        let results = if let Some(transaction) = transaction {
            transaction.apply(statements).await
        } else {
            driver
                .execute_mutation_batch(&target.database, statements)
                .await
                .map_err(|error| error.to_string())
        }
        .map_err(|error| GridSaveFailure::before_execution(total_statements, error))?;
        save_outcome(plan, total_statements, results)
    }
}

async fn grid_enum_values(
    driver: &dyn DatabaseDriver,
    db_type: DbType,
    database: &str,
    columns: &[ColumnInfo],
) -> Result<BTreeMap<usize, Vec<String>>, String> {
    match db_type {
        DbType::MySQL => Ok(columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| {
                let values = mysql_enum_values(&column.data_type)?;
                (!values.is_empty()).then_some((index, values))
            })
            .collect()),
        DbType::PostgreSQL => {
            let result = driver
                .execute_query(
                    database,
                    "SELECT t.typname, e.enumlabel FROM pg_catalog.pg_type t JOIN pg_catalog.pg_enum e ON e.enumtypid = t.oid ORDER BY t.typname, e.enumsortorder",
                )
                .await
                .map_err(|error| format!("Could not load PostgreSQL enum metadata: {error}"));
            postgres_enum_values_result(columns, result)
        }
        _ => Ok(BTreeMap::new()),
    }
}

fn postgres_enum_values_result(
    columns: &[ColumnInfo],
    result: Result<QueryResult, String>,
) -> Result<BTreeMap<usize, Vec<String>>, String> {
    result.map(|result| postgres_enum_values(columns, &result))
}

fn postgres_enum_values(
    columns: &[ColumnInfo],
    result: &QueryResult,
) -> BTreeMap<usize, Vec<String>> {
    let mut values_by_type = HashMap::<String, Vec<String>>::new();
    for row in &result.rows {
        let Some(enum_type) = row.first().and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(value) = row.get(1).and_then(serde_json::Value::as_str) else {
            continue;
        };
        values_by_type
            .entry(enum_type.to_string())
            .or_default()
            .push(value.to_string());
    }
    columns
        .iter()
        .enumerate()
        .filter_map(|(index, column)| {
            values_by_type
                .get(&column.data_type)
                .cloned()
                .map(|values| (index, values))
        })
        .collect()
}

fn mysql_enum_values(data_type: &str) -> Option<Vec<String>> {
    let body = data_type.trim().strip_prefix("enum(")?.strip_suffix(')')?;
    let mut values = Vec::new();
    let mut value = String::new();
    let mut characters = body.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '\'' {
            return None;
        }
        loop {
            match characters.next()? {
                '\\' => value.push(characters.next()?),
                '\'' if characters.peek() == Some(&'\'') => {
                    characters.next();
                    value.push('\'');
                }
                '\'' => break,
                character => value.push(character),
            }
        }
        values.push(std::mem::take(&mut value));
        match characters.next() {
            Some(',') => {}
            None => break,
            Some(_) => return None,
        }
    }
    Some(values)
}

fn grid_page_sql(
    db_type: DbType,
    table: &TableRef,
    page: u32,
    page_size: u32,
    filter: Option<&str>,
    sort: &[GridSort],
    columns: &[ColumnInfo],
) -> Result<String, String> {
    if page == 0 {
        return Err("grid page must be greater than zero".to_string());
    }
    if page_size == 0 {
        return Err("grid page size must be greater than zero".to_string());
    }
    let dialect = SqlDialect::new(db_type);
    let json_columns = projected_json_columns(db_type, columns);
    let mut select = "*".to_string();
    for (marker, column) in json_columns.iter().enumerate() {
        select.push_str(&format!(
            ", {} IS NULL AS {}",
            dialect.quote_identifier(&columns[*column].name)?,
            dialect.quote_identifier(&json_null_marker(marker))?,
        ));
    }
    let mut sql = format!("SELECT {select} FROM {}", dialect.quote_table_ref(table)?);
    if let Some(filter) = filter.map(str::trim).filter(|filter| !filter.is_empty()) {
        sql.push_str(" WHERE ");
        sql.push_str(filter);
        sql.push('\n');
    }
    if !sort.is_empty() {
        let order = sort
            .iter()
            .map(|sort| {
                let direction = match sort.direction {
                    GridSortDirection::Ascending => "ASC",
                    GridSortDirection::Descending => "DESC",
                };
                Ok(format!(
                    "{} {direction}",
                    dialect.quote_identifier(&sort.column)?
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        sql.push_str(" ORDER BY ");
        sql.push_str(&order.join(", "));
    } else if db_type == DbType::SQLServer {
        sql.push_str(" ORDER BY (SELECT NULL)");
    }
    let offset = u64::from(page - 1) * u64::from(page_size);
    if db_type == DbType::SQLServer {
        sql.push_str(&format!(
            " OFFSET {offset} ROWS FETCH NEXT {page_size} ROWS ONLY"
        ));
    } else {
        sql.push_str(&format!(" LIMIT {page_size} OFFSET {offset}"));
    }
    single_grid_statement(db_type, sql)
}

fn projected_json_columns(db_type: DbType, columns: &[ColumnInfo]) -> Vec<usize> {
    if !matches!(db_type, DbType::MySQL | DbType::PostgreSQL) {
        return Vec::new();
    }
    columns
        .iter()
        .enumerate()
        .filter_map(|(index, column)| {
            matches!(
                column.data_type.trim().to_ascii_lowercase().as_str(),
                "json" | "jsonb"
            )
            .then_some(index)
        })
        .collect()
}

fn json_null_marker(index: usize) -> String {
    format!("__astesia_json_sql_null_{index}")
}

fn normalize_grid_values(
    db_type: DbType,
    columns: &[ColumnInfo],
    result: &mut QueryResult,
) -> Result<(), String> {
    let json_columns = projected_json_columns(db_type, columns);
    if json_columns.is_empty() {
        return Ok(());
    }
    let expected = columns.len() + json_columns.len();
    if !result.columns.is_empty() && result.columns.len() != expected {
        return Err(format!(
            "grid query returned {} columns; expected {expected}",
            result.columns.len()
        ));
    }
    for (row_index, row) in result.rows.iter_mut().enumerate() {
        if row.len() != expected {
            return Err(format!(
                "grid query row {row_index} returned {} values; expected {expected}",
                row.len()
            ));
        }
        let markers = row.split_off(columns.len());
        for (column_index, marker) in json_columns.iter().copied().zip(markers) {
            let sql_null = match marker {
                serde_json::Value::Bool(value) => value,
                serde_json::Value::Number(value) => value.as_u64() == Some(1),
                serde_json::Value::String(value) => {
                    matches!(value.to_ascii_lowercase().as_str(), "1" | "true")
                }
                value => {
                    return Err(format!(
                        "grid JSON null marker for column {} has invalid value {value}",
                        columns[column_index].name
                    ));
                }
            };
            row[column_index] = if sql_null {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(row[column_index].to_string())
            };
        }
    }
    result.columns.truncate(columns.len());
    Ok(())
}

fn grid_count_sql(
    db_type: DbType,
    table: &TableRef,
    filter: Option<&str>,
) -> Result<String, String> {
    let dialect = SqlDialect::new(db_type);
    let mut sql = format!(
        "SELECT COUNT(*) AS total_rows FROM {}",
        dialect.quote_table_ref(table)?
    );
    if let Some(filter) = filter.map(str::trim).filter(|filter| !filter.is_empty()) {
        sql.push_str(" WHERE ");
        sql.push_str(filter);
    }
    single_grid_statement(db_type, sql)
}

fn single_grid_statement(db_type: DbType, sql: String) -> Result<String, String> {
    let mut statements = SqlScript::parse(db_type, &sql)
        .map_err(|error| error.to_string())?
        .into_statements();
    if statements.len() != 1 {
        return Err("grid filters must produce exactly one read statement".to_string());
    }
    Ok(statements.remove(0))
}

fn merge_column_metadata(result: &mut QueryResult, columns: Vec<ColumnInfo>) {
    if result.columns.is_empty() {
        result.columns = columns;
        return;
    }
    let columns = columns
        .into_iter()
        .map(|column| (column.name.clone(), column))
        .collect::<HashMap<_, _>>();
    for column in &mut result.columns {
        if let Some(metadata) = columns.get(&column.name) {
            *column = metadata.clone();
        }
    }
}

fn total_rows(result: &QueryResult) -> Option<u64> {
    let value = result.rows.first()?.first()?;
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn save_statement_count(plan: &GridSavePlan) -> usize {
    plan.updates.len() + plan.inserts.len() + usize::from(plan.delete.is_some())
}

fn save_statements(plan: &GridSavePlan) -> Result<Vec<String>, String> {
    let dialect = SqlDialect::new(plan.target.db_type);
    let mut statements = Vec::with_capacity(save_statement_count(plan));
    for update in &plan.updates {
        statements.push(dialect.build_update_row(
            &plan.table,
            &plan.primary_key_column,
            &update.primary_key_value,
            &update.column,
            &update.new_value,
        )?);
    }
    for insert in &plan.inserts {
        statements.push(dialect.build_insert_row(&plan.table, &insert.columns, &insert.values)?);
    }
    if let Some(delete) = &plan.delete {
        statements.push(dialect.build_delete_rows(
            &plan.table,
            &plan.primary_key_column,
            &delete.primary_key_values,
        )?);
    }
    Ok(statements)
}

fn save_outcome(
    plan: &GridSavePlan,
    total_statements: usize,
    results: Vec<StatementResult>,
) -> Result<GridSaveOutcome, GridSaveFailure> {
    let mut affected_rows = 0;
    for (index, result) in results.iter().enumerate() {
        if !result.success {
            return Err(GridSaveFailure::during_execution(
                index,
                total_statements,
                result
                    .error
                    .clone()
                    .unwrap_or_else(|| "Grid save statement failed".to_string()),
            ));
        }
        affected_rows += result.affected_rows;
    }
    if results.len() != total_statements {
        return Err(GridSaveFailure::during_execution(
            results.len(),
            total_statements,
            "Grid save stopped before every statement completed",
        ));
    }
    Ok(GridSaveOutcome {
        statements_executed: results.len(),
        changes_applied: plan.operation_count,
        affected_rows,
    })
}

#[cfg(test)]
mod tests {
    use crate::application::{
        ConnectionOutcome, GridCell, GridRowSelectionMode, GridSession, GridSessionStatus,
        QueryService, QueryTarget, DEFAULT_GRID_PAGE_SIZE,
    };
    use crate::connection_repository::SharedConnectionRepository;
    use crate::credential_vault::test_support::MemoryCredentialVault;
    use crate::db::{ConnectionConfig, DbType, TableRef};
    use serde_json::{json, Value};

    use super::*;

    fn column(name: &str, data_type: &str, nullable: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: data_type.to_string(),
            nullable,
            is_primary_key: false,
            default_value: None,
            comment: None,
        }
    }

    struct SqliteGrid {
        _directory: tempfile::TempDir,
        service: GridService,
        queries: QueryService,
        target: QueryTarget,
    }

    impl SqliteGrid {
        async fn new() -> Self {
            let directory = tempfile::TempDir::new().expect("tempdir");
            let database_path = directory.path().join("grid.sqlite3");
            std::fs::File::create(&database_path).expect("create sqlite database");
            let repository = SharedConnectionRepository::new(
                directory.path().join("connections.sqlite3"),
                MemoryCredentialVault::shared(),
            );
            repository
                .create(
                    ConnectionConfig {
                        id: "local".to_string(),
                        name: "Local".to_string(),
                        db_type: DbType::SQLite,
                        host: database_path.display().to_string(),
                        port: 0,
                        username: String::new(),
                        password: String::new(),
                        database: None,
                        color: None,
                    },
                    false,
                )
                .await
                .expect("create profile");
            let manager = ConnectionManager::new(repository);
            assert_eq!(
                manager.connect("local").await.expect("connect"),
                ConnectionOutcome::Succeeded
            );
            let (_, session_generation) = manager
                .driver_session("local")
                .await
                .expect("driver session");
            let queries = QueryService::new(manager.clone());
            for sql in [
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT UNIQUE NOT NULL, status TEXT)",
                "INSERT INTO users (id, name, status) VALUES (1, 'Ada', 'active')",
                "INSERT INTO users (id, name, status) VALUES (2, 'Lin', 'paused')",
                "INSERT INTO users (id, name, status) VALUES (3, 'Mira', 'active')",
            ] {
                queries
                    .execute("local", "main", sql)
                    .await
                    .expect("prepare grid fixture");
            }
            Self {
                _directory: directory,
                service: GridService::new(manager),
                queries,
                target: QueryTarget {
                    connection_id: "local".to_string(),
                    connection_name: "Local".to_string(),
                    database: "main".to_string(),
                    db_type: DbType::SQLite,
                    session_generation,
                },
            }
        }

        fn session(&self) -> GridSession {
            GridSession::new(
                self.target.clone(),
                TableRef::unqualified("users"),
                DEFAULT_GRID_PAGE_SIZE,
            )
            .unwrap()
        }

        async fn load(&self, session: &mut GridSession) {
            let request = session.begin_load().unwrap();
            let page = self.service.load(&request).await.unwrap();
            assert!(session.finish_load(&request, Ok(page)));
        }
    }

    #[tokio::test]
    async fn manual_grid_load_and_save_share_a_transaction() {
        let grid = SqliteGrid::new().await;
        let transaction = grid
            .service
            .begin_transaction(
                grid.target.clone(),
                crate::db::TransactionIsolation::Serializable,
            )
            .await
            .unwrap();
        let mut session = grid.session();
        let load = session.begin_load().unwrap();
        let page = grid
            .service
            .load_in(&load, Some(&transaction))
            .await
            .unwrap();
        session.finish_load(&load, Ok(page));
        session
            .stage_cell_value(GridCell { row: 0, column: 1 }, json!("Augusta"))
            .unwrap();
        let save = session.begin_save().unwrap();
        grid.service
            .save_in(&save, Some(&transaction))
            .await
            .unwrap();
        session.finish_save(&save, Ok(()));
        assert!(transaction.has_pending_changes());
        assert!(transaction.recovery_sql().contains("Augusta"));
        let load = session.begin_load().unwrap();
        let page = grid
            .service
            .load_in(&load, Some(&transaction))
            .await
            .unwrap();
        assert_eq!(page.rows[0][1], json!("Augusta"));
        let outside = grid.service.load(&load).await.unwrap();
        assert_eq!(outside.rows[0][1], json!("Ada"));
        transaction.finish(true).await.unwrap();
        assert!(!transaction.has_pending_changes());
        let committed = grid.service.load(&load).await.unwrap();
        assert_eq!(committed.rows[0][1], json!("Augusta"));
    }

    #[test]
    fn page_queries_quote_sort_columns_and_use_engine_pagination() {
        let table = TableRef::qualified("audit", "events");
        let sort = vec![GridSort {
            column: "created at".to_string(),
            direction: GridSortDirection::Descending,
        }];
        assert_eq!(
            grid_page_sql(
                DbType::PostgreSQL,
                &table,
                3,
                50,
                Some("kind = 'login'"),
                &sort,
                &[],
            )
            .unwrap(),
            "SELECT * FROM \"audit\".\"events\" WHERE kind = 'login'\n ORDER BY \"created at\" DESC LIMIT 50 OFFSET 100"
        );
        assert_eq!(
            grid_page_sql(DbType::SQLServer, &table, 2, 25, None, &[], &[]).unwrap(),
            "SELECT * FROM [audit].[events] ORDER BY (SELECT NULL) OFFSET 25 ROWS FETCH NEXT 25 ROWS ONLY"
        );
        assert_eq!(
            grid_count_sql(DbType::SQLite, &TableRef::unqualified("users"), None).unwrap(),
            "SELECT COUNT(*) AS total_rows FROM \"users\""
        );
        assert!(grid_page_sql(
            DbType::SQLite,
            &TableRef::unqualified("users"),
            1,
            100,
            Some("1 = 1; DELETE FROM users"),
            &[],
            &[],
        )
        .is_err());
        assert!(grid_page_sql(
            DbType::SQLite,
            &TableRef::unqualified("users"),
            1,
            100,
            Some("name = ';'"),
            &[],
            &[],
        )
        .is_ok());
    }

    #[test]
    fn grid_query_preserves_sql_null_separately_from_json_null() {
        let columns = vec![
            column("id", "bigint", false),
            column("price", "numeric", false),
            column("object", "jsonb", false),
            column("scalar", "json", false),
            column("json_null", "jsonb", true),
            column("sql_null", "jsonb", true),
        ];
        let sql = grid_page_sql(
            DbType::PostgreSQL,
            &TableRef::qualified("public", "items"),
            1,
            100,
            None,
            &[],
            &columns,
        )
        .unwrap();
        assert_eq!(
            sql,
            "SELECT *, \"object\" IS NULL AS \"__astesia_json_sql_null_0\", \"scalar\" IS NULL AS \"__astesia_json_sql_null_1\", \"json_null\" IS NULL AS \"__astesia_json_sql_null_2\", \"sql_null\" IS NULL AS \"__astesia_json_sql_null_3\" FROM \"public\".\"items\" LIMIT 100 OFFSET 0"
        );

        let mut result = QueryResult {
            columns: columns
                .iter()
                .cloned()
                .chain((0..4).map(|index| column(&json_null_marker(index), "bool", false)))
                .collect(),
            rows: vec![vec![
                json!(1),
                json!("12.50"),
                json!({"ok": true}),
                json!("hello"),
                Value::Null,
                Value::Null,
                json!(false),
                json!(0),
                json!("false"),
                json!(true),
            ]],
            ..Default::default()
        };
        normalize_grid_values(DbType::PostgreSQL, &columns, &mut result).unwrap();
        assert_eq!(result.columns.len(), columns.len());
        assert_eq!(result.columns[0].name, "id");
        assert_eq!(result.columns[5].name, "sql_null");
        assert_eq!(
            result.rows[0],
            vec![
                json!(1),
                json!("12.50"),
                json!("{\"ok\":true}"),
                json!("\"hello\""),
                json!("null"),
                Value::Null,
            ]
        );
    }

    #[test]
    fn grid_enum_metadata_supports_mysql_and_postgres_types() {
        assert_eq!(
            mysql_enum_values("enum('active','it\\'s paused','archived')"),
            Some(vec![
                "active".to_string(),
                "it's paused".to_string(),
                "archived".to_string(),
            ])
        );
        let columns = vec![
            column("id", "bigint", false),
            column("status", "account_status", false),
        ];
        let result = QueryResult {
            rows: vec![
                vec![json!("account_status"), json!("active")],
                vec![json!("account_status"), json!("paused")],
            ],
            ..Default::default()
        };
        assert_eq!(
            postgres_enum_values(&columns, &result).get(&1),
            Some(&vec!["active".to_string(), "paused".to_string()])
        );
        assert_eq!(
            postgres_enum_values_result(&columns, Err("catalog unavailable".to_string())),
            Err("catalog unavailable".to_string())
        );
    }

    #[tokio::test]
    async fn grid_service_loads_filters_sorts_and_merges_authoritative_metadata() {
        let grid = SqliteGrid::new().await;
        let mut session = grid.session();
        grid.load(&mut session).await;

        assert!(matches!(session.status(), GridSessionStatus::Ready));
        let page = session.page().expect("ready grid has a page");
        assert_eq!(page.total_rows, Some(3));
        assert!(page.columns[0].is_primary_key);

        session
            .set_query_options(
                Some("status = 'active'".to_string()),
                vec![GridSort {
                    column: "name".to_string(),
                    direction: GridSortDirection::Descending,
                }],
            )
            .unwrap();
        let request = session.begin_load().unwrap();
        let page = grid.service.load(&request).await.unwrap();
        assert_eq!(page.total_rows, Some(2));
        assert_eq!(page.rows[0][1], json!("Mira"));
        assert_eq!(page.rows[1][1], json!("Ada"));
    }

    #[tokio::test]
    async fn successful_save_executes_the_plan_and_forces_a_reload() {
        let grid = SqliteGrid::new().await;
        let mut session = grid.session();
        grid.load(&mut session).await;
        session
            .stage_cell_value(GridCell { row: 0, column: 1 }, json!("Augusta"))
            .unwrap();
        let draft_id = session.stage_insert().unwrap();
        session.set_draft_value(draft_id, 0, json!(4)).unwrap();
        session.set_draft_value(draft_id, 1, json!("Dora")).unwrap();
        session
            .set_draft_value(draft_id, 2, json!("active"))
            .unwrap();
        session
            .select_row(1, GridRowSelectionMode::Replace)
            .unwrap();
        session.stage_delete_selection().unwrap();

        let request = session.begin_save().unwrap();
        let outcome = grid.service.save(&request).await.unwrap();
        assert_eq!(outcome.statements_executed, 3);
        assert_eq!(outcome.changes_applied, 3);
        assert_eq!(outcome.affected_rows, 3);
        assert!(session.finish_save(&request, Ok(())));
        assert!(matches!(session.status(), GridSessionStatus::Idle));

        grid.load(&mut session).await;
        assert!(matches!(session.status(), GridSessionStatus::Ready));
        let page = session.page().expect("ready grid has a page");
        assert_eq!(page.total_rows, Some(3));
        assert_eq!(page.rows[0][1], json!("Augusta"));
        assert_eq!(page.rows[1][0], json!(3));
        assert_eq!(page.rows[2][1], json!("Dora"));
    }

    #[tokio::test]
    async fn failed_save_rolls_back_and_keeps_the_session_editable() {
        let grid = SqliteGrid::new().await;
        let mut session = grid.session();
        grid.load(&mut session).await;
        session
            .stage_cell_value(GridCell { row: 0, column: 1 }, json!("Shared"))
            .unwrap();
        session
            .stage_cell_value(GridCell { row: 1, column: 1 }, json!("Shared"))
            .unwrap();

        let request = session.begin_save().unwrap();
        let error = grid.service.save(&request).await.unwrap_err();
        assert_eq!(error.completed_statements, 1);
        assert_eq!(error.total_statements, 2);
        assert!(session.finish_save(&request, Err(error)));
        assert!(session.has_changes());
        assert!(matches!(
            session.status(),
            GridSessionStatus::SaveFailed { .. }
        ));

        let result = grid
            .queries
            .execute("local", "main", "SELECT name FROM users ORDER BY id")
            .await
            .unwrap();
        assert_eq!(result.rows[0][0], json!("Ada"));
        assert_eq!(result.rows[1][0], json!("Lin"));

        assert!(session.undo());
        let retry = session.begin_save().unwrap();
        let outcome = grid.service.save(&retry).await.unwrap();
        assert_eq!(outcome.changes_applied, 1);
        assert!(session.finish_save(&retry, Ok(())));
    }

    #[tokio::test]
    async fn stale_target_generation_is_rejected_before_loading() {
        let grid = SqliteGrid::new().await;
        let mut target = grid.target.clone();
        target.session_generation += 1;
        let mut session = GridSession::new(
            target,
            TableRef::unqualified("users"),
            DEFAULT_GRID_PAGE_SIZE,
        )
        .unwrap();
        let request = session.begin_load().unwrap();

        assert!(matches!(
            grid.service.load(&request).await,
            Err(GridLoadError::SessionChanged { .. })
        ));
    }
}
