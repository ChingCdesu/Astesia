mod dialects;
mod scope;
use scope::CompletionScope;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex as SyncMutex},
};

use async_trait::async_trait;
use tokio::sync::{Mutex, OnceCell};

use super::{CatalogService, QueryTarget};
use crate::db::{ColumnInfo, DbType, SqlDialect, TableInfo, TableRef};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QueryCompletionRequest {
    pub(crate) target: QueryTarget,
    pub(crate) text_before_cursor: String,
    pub(crate) text_after_cursor: String,
    pub(crate) schema: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum QueryCompletionKind {
    Table,
    Schema,
    Column,
    Keyword,
    Function,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QueryCompletion {
    pub(crate) label: String,
    pub(crate) new_text: String,
    pub(crate) detail: String,
    pub(crate) kind: QueryCompletionKind,
}

const MAX_CACHED_TARGETS: usize = 16;
const MAX_CACHED_COLUMN_TABLES: usize = 64;

type CachedTargets = VecDeque<(CompletionTargetKey, Arc<TargetCatalog>)>;
type CachedColumns = VecDeque<(TableRef, Arc<OnceCell<Arc<Vec<ColumnInfo>>>>)>;

#[derive(Clone)]
pub(crate) struct QueryCompletionService {
    catalog: Arc<dyn CompletionCatalog>,
    targets: Arc<SyncMutex<CachedTargets>>,
}

impl QueryCompletionService {
    pub(crate) fn new(catalog: CatalogService) -> Self {
        Self {
            catalog: Arc::new(catalog),
            targets: Arc::default(),
        }
    }

    #[cfg(test)]
    fn with_catalog(catalog: Arc<dyn CompletionCatalog>) -> Self {
        Self {
            catalog,
            targets: Arc::default(),
        }
    }

    pub(crate) async fn complete(&self, request: QueryCompletionRequest) -> Vec<QueryCompletion> {
        if !request.target.db_type.capabilities().sql {
            return Vec::new();
        }

        let mut scope = CompletionScope::parse(&request.text_before_cursor, request.target.db_type);
        scope.include_following_tables(
            &request.text_before_cursor,
            &request.text_after_cursor,
            request.target.db_type,
        );
        if scope.suppressed {
            return Vec::new();
        }
        let mut fallback = dialects::completions(request.target.db_type);
        if scope.relation {
            fallback.retain(|item| item.kind == QueryCompletionKind::Keyword);
        }
        if !scope.relation && scope.qualifier.is_none() && scope.tables.is_empty() {
            return fallback;
        }
        let catalog = self.target_catalog(&request.target);
        let Ok(tables) = catalog
            .tables
            .get_or_try_init(|| self.catalog.tables(&request.target))
            .await
        else {
            return fallback;
        };

        if let Some(qualifier) = &scope.qualifier {
            let resolved = scope
                .tables
                .iter()
                .find(|(_, alias)| {
                    alias
                        .as_ref()
                        .is_some_and(|alias| alias.eq_ignore_ascii_case(qualifier))
                })
                .map(|(name, _)| name.as_str())
                .unwrap_or(qualifier);
            if !scope.relation {
                let Some(table) = find_table(tables, resolved, request.schema.as_deref()) else {
                    return Vec::new();
                };
                let Ok(columns) = catalog
                    .columns(self.catalog.as_ref(), &request.target, &table.reference)
                    .await
                else {
                    return Vec::new();
                };
                return column_completions(request.target.db_type, &columns);
            }
            let schema_tables = tables
                .iter()
                .filter(|table| {
                    table
                        .reference
                        .schema()
                        .is_some_and(|schema| schema.eq_ignore_ascii_case(qualifier))
                })
                .collect::<Vec<_>>();
            return schema_table_completions(request.target.db_type, schema_tables);
        }

        let mut completions = Vec::new();
        if scope.relation {
            completions.extend(table_completions(
                request.target.db_type,
                tables,
                request.schema.as_deref(),
            ));
            completions.extend(schema_completions(request.target.db_type, tables));
        } else {
            let qualify_columns = scope.tables.len() > 1 && !scope.using_columns;
            let mut column_sources = HashMap::<String, usize>::new();
            for (name, alias) in &scope.tables {
                let Some(table) = find_table(tables, name, request.schema.as_deref()) else {
                    continue;
                };
                if let Ok(columns) = catalog
                    .columns(self.catalog.as_ref(), &request.target, &table.reference)
                    .await
                {
                    let mut candidates = column_completions(request.target.db_type, &columns);
                    if scope.using_columns {
                        let names = candidates
                            .iter()
                            .map(|item| item.label.clone())
                            .collect::<HashSet<_>>();
                        for name in names {
                            *column_sources.entry(name).or_default() += 1;
                        }
                    }
                    if qualify_columns {
                        let qualifier = alias.as_deref().unwrap_or(name);
                        let insertion_qualifier = alias
                            .as_deref()
                            .map(|alias| completion_identifier(request.target.db_type, alias))
                            .unwrap_or_else(|| {
                                completion_table_name(request.target.db_type, &table.reference)
                            });
                        for candidate in &mut candidates {
                            candidate.label = format!("{qualifier}.{}", candidate.label);
                            candidate.new_text =
                                format!("{insertion_qualifier}.{}", candidate.new_text);
                        }
                    }
                    completions.extend(candidates);
                }
            }
            if scope.using_columns {
                completions
                    .retain(|item| column_sources.get(&item.label) == Some(&scope.tables.len()));
                return deduplicate(completions);
            }
        }
        completions.append(&mut fallback);
        deduplicate(completions)
    }

    pub(crate) fn invalidate_connection(&self, connection_id: &str) {
        self.targets
            .lock()
            .expect("completion target cache")
            .retain(|(key, _)| key.connection_id != connection_id);
    }

    pub(crate) fn invalidate_session(&self, connection_id: &str, generation: u64) {
        self.targets
            .lock()
            .expect("completion target cache")
            .retain(|(key, _)| {
                key.connection_id != connection_id || key.session_generation != generation
            });
    }

    pub(crate) fn retain_sessions(&self, snapshot: &super::ConnectionWorkspaceSnapshot) {
        self.targets
            .lock()
            .expect("completion target cache")
            .retain(|(key, _)| {
                snapshot.profiles.iter().any(|entry| {
                    entry.profile.id == key.connection_id
                        && entry.session.generation == Some(key.session_generation)
                })
            });
    }

    fn target_catalog(&self, target: &QueryTarget) -> Arc<TargetCatalog> {
        let key = CompletionTargetKey::from(target);
        let mut targets = self.targets.lock().expect("completion target cache");
        targets.retain(|(candidate, _)| {
            candidate.connection_id != key.connection_id
                || candidate.session_generation == key.session_generation
        });
        let entry = targets
            .iter()
            .position(|(candidate, _)| *candidate == key)
            .and_then(|index| targets.remove(index))
            .unwrap_or_else(|| (key, Arc::new(TargetCatalog::default())));
        let catalog = entry.1.clone();
        targets.push_back(entry);
        while targets.len() > MAX_CACHED_TARGETS {
            targets.pop_front();
        }
        catalog
    }
}

#[async_trait]
trait CompletionCatalog: Send + Sync {
    async fn tables(&self, target: &QueryTarget) -> Result<Vec<TableInfo>, String>;

    async fn columns(
        &self,
        target: &QueryTarget,
        table: &TableRef,
    ) -> Result<Vec<ColumnInfo>, String>;
}

#[async_trait]
impl CompletionCatalog for CatalogService {
    async fn tables(&self, target: &QueryTarget) -> Result<Vec<TableInfo>, String> {
        self.tables(&target.connection_id, &target.database).await
    }

    async fn columns(
        &self,
        target: &QueryTarget,
        table: &TableRef,
    ) -> Result<Vec<ColumnInfo>, String> {
        self.columns(&target.connection_id, &target.database, table)
            .await
    }
}

#[derive(Debug, Eq, Hash, PartialEq)]
struct CompletionTargetKey {
    connection_id: String,
    database: String,
    db_type: DbType,
    session_generation: u64,
}

impl From<&QueryTarget> for CompletionTargetKey {
    fn from(target: &QueryTarget) -> Self {
        Self {
            connection_id: target.connection_id.clone(),
            database: target.database.clone(),
            db_type: target.db_type,
            session_generation: target.session_generation,
        }
    }
}

#[derive(Default)]
struct TargetCatalog {
    tables: OnceCell<Vec<TableInfo>>,
    columns: Mutex<CachedColumns>,
}

impl TargetCatalog {
    async fn columns(
        &self,
        catalog: &dyn CompletionCatalog,
        target: &QueryTarget,
        table: &TableRef,
    ) -> Result<Arc<Vec<ColumnInfo>>, String> {
        let cell = {
            let mut columns = self.columns.lock().await;
            let entry = columns
                .iter()
                .position(|(candidate, _)| candidate == table)
                .and_then(|index| columns.remove(index))
                .unwrap_or_else(|| (table.clone(), Arc::new(OnceCell::new())));
            let cell = entry.1.clone();
            columns.push_back(entry);
            while columns.len() > MAX_CACHED_COLUMN_TABLES {
                columns.pop_front();
            }
            cell
        };
        cell.get_or_try_init(|| async { catalog.columns(target, table).await.map(Arc::new) })
            .await
            .cloned()
    }
}

fn find_table<'a>(
    tables: &'a [TableInfo],
    qualifier: &str,
    schema: Option<&str>,
) -> Option<&'a TableInfo> {
    tables.iter().find(|table| {
        (table.reference.name().eq_ignore_ascii_case(qualifier)
            && schema.is_none_or(|preferred| {
                table
                    .reference
                    .schema()
                    .is_some_and(|schema| schema == preferred)
            }))
            || table.reference.schema().is_some_and(|schema| {
                format!("{schema}.{}", table.reference.name()).eq_ignore_ascii_case(qualifier)
            })
    })
}

fn table_completions(
    db_type: DbType,
    tables: &[TableInfo],
    schema: Option<&str>,
) -> Vec<QueryCompletion> {
    tables
        .iter()
        .flat_map(|table| {
            let name = table.reference.name();
            let qualified_label = table
                .reference
                .schema()
                .map(|schema| format!("{schema}.{name}"));
            let qualified = QueryCompletion {
                label: qualified_label.clone().unwrap_or_else(|| name.to_string()),
                new_text: completion_table_name(db_type, &table.reference),
                detail: "table".to_string(),
                kind: QueryCompletionKind::Table,
            };
            let unqualified = qualified_label
                .filter(|_| schema.is_none_or(|schema| table.reference.schema() == Some(schema)))
                .map(|label| QueryCompletion {
                    label: name.to_string(),
                    new_text: completion_identifier(db_type, name),
                    detail: label,
                    kind: QueryCompletionKind::Table,
                });
            std::iter::once(qualified).chain(unqualified)
        })
        .collect()
}

fn schema_table_completions(db_type: DbType, tables: Vec<&TableInfo>) -> Vec<QueryCompletion> {
    tables
        .into_iter()
        .map(|table| QueryCompletion {
            label: table.reference.name().to_string(),
            new_text: completion_identifier(db_type, table.reference.name()),
            detail: table.reference.to_string(),
            kind: QueryCompletionKind::Table,
        })
        .collect()
}

fn schema_completions(db_type: DbType, tables: &[TableInfo]) -> Vec<QueryCompletion> {
    let schemas = tables
        .iter()
        .filter_map(|table| table.reference.schema())
        .collect::<HashSet<_>>();
    schemas
        .into_iter()
        .map(|schema| QueryCompletion {
            label: schema.to_string(),
            new_text: completion_identifier(db_type, schema),
            detail: "schema".to_string(),
            kind: QueryCompletionKind::Schema,
        })
        .collect()
}

fn column_completions<T>(db_type: DbType, columns: &[T]) -> Vec<QueryCompletion>
where
    T: std::borrow::Borrow<ColumnInfo>,
{
    columns
        .iter()
        .map(std::borrow::Borrow::borrow)
        .map(|column| QueryCompletion {
            label: column.name.clone(),
            new_text: completion_identifier(db_type, &column.name),
            detail: column.data_type.clone(),
            kind: QueryCompletionKind::Column,
        })
        .collect()
}

fn completion_table_name(db_type: DbType, table: &TableRef) -> String {
    let name = completion_identifier(db_type, table.name());
    table
        .schema()
        .map(|schema| format!("{}.{}", completion_identifier(db_type, schema), name))
        .unwrap_or(name)
}

fn completion_identifier(db_type: DbType, identifier: &str) -> String {
    if identifier.chars().enumerate().all(|(index, character)| {
        character == '_'
            || character.is_ascii_alphanumeric() && (index > 0 || character.is_ascii_alphabetic())
    }) {
        return identifier.to_string();
    }
    SqlDialect::new(db_type)
        .quote_identifier(identifier)
        .unwrap_or_else(|_| identifier.to_string())
}

fn deduplicate(completions: Vec<QueryCompletion>) -> Vec<QueryCompletion> {
    let mut seen = HashSet::new();
    completions
        .into_iter()
        .filter(|completion| seen.insert((completion.kind, completion.label.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct TestCatalog {
        table_loads: AtomicUsize,
        tables: Vec<TableInfo>,
        columns: HashMap<TableRef, Vec<ColumnInfo>>,
    }

    #[async_trait]
    impl CompletionCatalog for TestCatalog {
        async fn tables(&self, _: &QueryTarget) -> Result<Vec<TableInfo>, String> {
            self.table_loads.fetch_add(1, Ordering::SeqCst);
            Ok(self.tables.clone())
        }

        async fn columns(
            &self,
            _: &QueryTarget,
            table: &TableRef,
        ) -> Result<Vec<ColumnInfo>, String> {
            Ok(self.columns.get(table).cloned().unwrap_or_default())
        }
    }

    fn target(db_type: DbType, generation: u64) -> QueryTarget {
        QueryTarget {
            connection_id: "connection".to_string(),
            connection_name: "Connection".to_string(),
            database: "database".to_string(),
            db_type,
            session_generation: generation,
        }
    }

    struct PendingCatalog;
    #[async_trait]
    impl CompletionCatalog for PendingCatalog {
        async fn tables(&self, _: &QueryTarget) -> Result<Vec<TableInfo>, String> {
            std::future::pending().await
        }
        async fn columns(&self, _: &QueryTarget, _: &TableRef) -> Result<Vec<ColumnInfo>, String> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn keyword_completion_does_not_wait_for_database_metadata() {
        let service = QueryCompletionService::with_catalog(Arc::new(PendingCatalog));
        for sql in ["SEL", "SELECT ", "SELECT COU"] {
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(20),
                service.complete(QueryCompletionRequest {
                    target: target(DbType::PostgreSQL, 1),
                    text_before_cursor: sql.into(),
                    text_after_cursor: String::new(),
                    schema: None,
                }),
            )
            .await
            .expect("local completions must not await the database");
            assert!(result.iter().any(|item| item.label == "SELECT"));
        }
    }

    #[tokio::test]
    async fn target_budget_evicts_old_catalogs_and_disconnect_drops_session_data() {
        let catalog = Arc::new(TestCatalog {
            table_loads: AtomicUsize::new(0),
            tables: Vec::new(),
            columns: HashMap::new(),
        });
        let service = QueryCompletionService::with_catalog(catalog);
        let first = target(DbType::SQLite, 1);
        let first_cache = service.target_catalog(&first);
        let first_weak = Arc::downgrade(&first_cache);
        drop(first_cache);
        for index in 0..MAX_CACHED_TARGETS {
            let mut next = first.clone();
            next.database = format!("database-{index}");
            service.target_catalog(&next);
        }
        assert!(first_weak.upgrade().is_none());
        assert_eq!(service.targets.lock().unwrap().len(), MAX_CACHED_TARGETS);
        service.invalidate_session(&first.connection_id, 1);
        assert!(service.targets.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn column_budget_evicts_old_table_metadata() {
        let catalog = TestCatalog {
            table_loads: AtomicUsize::new(0),
            tables: Vec::new(),
            columns: HashMap::new(),
        };
        let cached = TargetCatalog::default();
        let target = target(DbType::SQLite, 1);
        let first = cached
            .columns(&catalog, &target, &TableRef::unqualified("first"))
            .await
            .unwrap();
        let weak = Arc::downgrade(&first);
        drop(first);
        for index in 0..MAX_CACHED_COLUMN_TABLES {
            cached
                .columns(
                    &catalog,
                    &target,
                    &TableRef::unqualified(format!("table-{index}")),
                )
                .await
                .unwrap();
        }
        assert!(weak.upgrade().is_none());
        assert_eq!(cached.columns.lock().await.len(), MAX_CACHED_COLUMN_TABLES);
    }

    fn table(schema: Option<&str>, name: &str) -> TableInfo {
        TableInfo {
            reference: TableRef::from_parts(schema.map(str::to_string), name.to_string()),
            row_count: None,
            comment: None,
        }
    }

    fn column(name: &str, data_type: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: data_type.to_string(),
            nullable: false,
            is_primary_key: false,
            default_value: None,
            comment: None,
        }
    }

    #[tokio::test]
    async fn completes_dialects_tables_schemas_and_quoted_identifiers() {
        let catalog = Arc::new(TestCatalog {
            table_loads: AtomicUsize::new(0),
            tables: vec![table(Some("sales data"), "order items")],
            columns: HashMap::new(),
        });
        let service = QueryCompletionService::with_catalog(catalog.clone());
        let completions = service
            .complete(QueryCompletionRequest {
                target: target(DbType::PostgreSQL, 1),
                text_before_cursor: "SELECT * FROM ".to_string(),
                text_after_cursor: String::new(),
                schema: None,
            })
            .await;

        assert!(completions
            .iter()
            .any(|item| { item.kind == QueryCompletionKind::Keyword && item.label == "SELECT" }));
        assert!(completions.iter().any(|item| {
            item.kind == QueryCompletionKind::Table
                && item.new_text == "\"sales data\".\"order items\""
        }));
        assert!(completions.iter().any(|item| {
            item.kind == QueryCompletionKind::Schema && item.new_text == "\"sales data\""
        }));
        assert_eq!(catalog.table_loads.load(Ordering::SeqCst), 1);

        service
            .complete(QueryCompletionRequest {
                target: target(DbType::PostgreSQL, 1),
                text_before_cursor: "SELECT ".to_string(),
                text_after_cursor: String::new(),
                schema: None,
            })
            .await;
        assert_eq!(catalog.table_loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dot_completion_loads_columns_and_new_sessions_invalidate_catalogs() {
        let users = table(None, "users");
        let catalog = Arc::new(TestCatalog {
            table_loads: AtomicUsize::new(0),
            tables: vec![users.clone()],
            columns: HashMap::from([(
                users.reference.clone(),
                vec![column("display name", "text")],
            )]),
        });
        let service = QueryCompletionService::with_catalog(catalog.clone());
        let columns = service
            .complete(QueryCompletionRequest {
                target: target(DbType::MySQL, 1),
                text_before_cursor: "SELECT users.".to_string(),
                text_after_cursor: String::new(),
                schema: None,
            })
            .await;
        assert_eq!(
            columns,
            vec![QueryCompletion {
                label: "display name".to_string(),
                new_text: "`display name`".to_string(),
                detail: "text".to_string(),
                kind: QueryCompletionKind::Column,
            }]
        );

        service
            .complete(QueryCompletionRequest {
                target: target(DbType::MySQL, 2),
                text_before_cursor: "SELECT * FROM ".to_string(),
                text_after_cursor: String::new(),
                schema: None,
            })
            .await;
        assert_eq!(catalog.table_loads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn candidates_follow_clauses_and_resolve_alias_columns() {
        let users = table(Some("public"), "users");
        let orders = table(Some("public"), "orders");
        let service = QueryCompletionService::with_catalog(Arc::new(TestCatalog {
            table_loads: AtomicUsize::new(0),
            tables: vec![users.clone(), orders.clone()],
            columns: HashMap::from([
                (users.reference, vec![column("email", "text")]),
                (orders.reference, vec![column("total", "numeric")]),
            ]),
        }));
        for sql in [
            "SELECT ",
            "SELECT us",
            "SELECT COUNT(",
            "SELECT * FROM public.users u WHERE ",
            "SELECT * FROM public.users u ORDER BY ",
        ] {
            let result = service
                .complete(QueryCompletionRequest {
                    target: target(DbType::PostgreSQL, 1),
                    text_before_cursor: sql.into(),
                    text_after_cursor: String::new(),
                    schema: None,
                })
                .await;
            assert!(
                !result.iter().any(|item| matches!(
                    item.kind,
                    QueryCompletionKind::Table | QueryCompletionKind::Schema
                )),
                "{sql}"
            );
            assert!(
                !result.iter().any(|item| item.label == "total"),
                "unrelated cached column in {sql}"
            );
            if sql.contains("FROM") {
                assert!(result.iter().any(|item| item.label == "email"), "{sql}");
            }
        }
        for sql in [
            "SELECT * FROM ",
            "SELECT * FROM us",
            "SELECT * FROM public.",
            "SELECT * FROM public.us",
            "SELECT * FROM users JOIN ",
            "SELECT * FROM users, ",
            "UPDATE ",
            "INSERT INTO ",
        ] {
            let result = service
                .complete(QueryCompletionRequest {
                    target: target(DbType::PostgreSQL, 1),
                    text_before_cursor: sql.into(),
                    text_after_cursor: String::new(),
                    schema: None,
                })
                .await;
            assert!(
                result
                    .iter()
                    .any(|item| item.kind == QueryCompletionKind::Table),
                "{sql}"
            );
            assert!(
                !result.iter().any(|item| matches!(
                    item.kind,
                    QueryCompletionKind::Column | QueryCompletionKind::Function
                )),
                "{sql}"
            );
        }
        let result = service
            .complete(QueryCompletionRequest {
                target: target(DbType::PostgreSQL, 1),
                text_before_cursor: "SELECT * FROM public.users AS u WHERE u.".into(),
                text_after_cursor: String::new(),
                schema: None,
            })
            .await;
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "email");
    }

    #[tokio::test]
    async fn select_columns_use_tables_and_aliases_after_cursor() {
        let users = table(Some("public"), "users");
        let service = QueryCompletionService::with_catalog(Arc::new(TestCatalog {
            table_loads: AtomicUsize::new(0),
            tables: vec![users.clone()],
            columns: HashMap::from([(users.reference, vec![column("email", "text")])]),
        }));
        for prefix in ["SELECT ", "SELECT em", "SELECT u.", "SELECT COUNT("] {
            let suffix = if prefix.ends_with('(') {
                ") FROM public.users u"
            } else {
                " FROM public.users u"
            };
            let result = service
                .complete(QueryCompletionRequest {
                    target: target(DbType::PostgreSQL, 1),
                    text_before_cursor: prefix.into(),
                    text_after_cursor: suffix.into(),
                    schema: None,
                })
                .await;
            assert!(
                result.iter().any(|item| item.label == "email"),
                "{prefix}|{suffix}"
            );
            assert!(!result
                .iter()
                .any(|item| item.kind == QueryCompletionKind::Table));
        }
    }

    #[tokio::test]
    async fn joins_offer_distinct_qualified_columns_and_resolve_each_alias() {
        let users = table(None, "users");
        let orders = table(None, "orders");
        let service = QueryCompletionService::with_catalog(Arc::new(TestCatalog {
            table_loads: AtomicUsize::new(0),
            tables: vec![users.clone(), orders.clone()],
            columns: HashMap::from([
                (
                    users.reference,
                    vec![column("id", "int"), column("email", "text")],
                ),
                (
                    orders.reference,
                    vec![column("id", "int"), column("user_id", "int")],
                ),
            ]),
        }));
        for (before, after, expected) in [
            ("SELECT * FROM users JOIN orders USING (", ")", vec!["id"]),
            (
                "SELECT ",
                " FROM users u JOIN orders o ON u.id = o.user_id",
                vec!["u.id", "u.email", "o.id", "o.user_id"],
            ),
            (
                "SELECT * FROM users u LEFT JOIN orders o ON ",
                "",
                vec!["u.id", "u.email", "o.id", "o.user_id"],
            ),
            (
                "SELECT u.",
                " FROM users u JOIN orders o ON u.id = o.user_id",
                vec!["id", "email"],
            ),
            (
                "SELECT * FROM users u JOIN orders o ON o.",
                "",
                vec!["id", "user_id"],
            ),
            (
                "SELECT ",
                " FROM users u JOIN users manager ON u.id = manager.id",
                vec!["u.id", "u.email", "manager.id", "manager.email"],
            ),
        ] {
            let result = service
                .complete(QueryCompletionRequest {
                    target: target(DbType::PostgreSQL, 1),
                    text_before_cursor: before.into(),
                    text_after_cursor: after.into(),
                    schema: None,
                })
                .await;
            let columns = result
                .iter()
                .filter(|item| item.kind == QueryCompletionKind::Column)
                .collect::<Vec<_>>();
            assert_eq!(
                columns
                    .iter()
                    .map(|item| item.label.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{before}|{after}"
            );
            assert!(columns.iter().all(|item| item.label == item.new_text));
        }
    }

    #[tokio::test]
    async fn selected_schema_controls_unqualified_completion() {
        let public = table(Some("public"), "users");
        let audit = table(Some("audit"), "users");
        let service = QueryCompletionService::with_catalog(Arc::new(TestCatalog {
            table_loads: AtomicUsize::new(0),
            tables: vec![public.clone(), audit.clone()],
            columns: HashMap::from([
                (public.reference, vec![column("email", "text")]),
                (audit.reference, vec![column("event", "text")]),
            ]),
        }));
        for (schema, expected) in [("public", "email"), ("audit", "event")] {
            let result = service
                .complete(QueryCompletionRequest {
                    target: target(DbType::PostgreSQL, 1),
                    text_before_cursor: "SELECT ".into(),
                    text_after_cursor: " FROM users".into(),
                    schema: Some(schema.into()),
                })
                .await;
            let columns = result
                .iter()
                .filter(|item| item.kind == QueryCompletionKind::Column)
                .collect::<Vec<_>>();
            assert_eq!(columns.len(), 1);
            assert_eq!(columns[0].label, expected);
        }
    }

    #[test]
    fn scope_ignores_comments_literals_and_other_statements() {
        for sql in [
            "SELECT 'FROM users' ",
            "SELECT /* FROM users */ ",
            "SELECT * FROM users; SELECT ",
            "SELECT * FROM users WHERE id IN (SELECT ",
        ] {
            let scope = CompletionScope::parse(sql, DbType::PostgreSQL);
            assert!(!scope.relation, "{sql}");
            assert!(scope.tables.is_empty(), "{sql}");
        }
        for sql in ["SELECT 'unfinished", "SELECT -- FROM users"] {
            assert!(CompletionScope::parse(sql, DbType::PostgreSQL).suppressed);
        }
    }

    #[test]
    fn every_sql_engine_keeps_its_legacy_dialect_vocabulary() {
        for (db_type, expected) in [
            (DbType::MySQL, "SHOW"),
            (DbType::PostgreSQL, "RETURNING"),
            (DbType::SQLite, "PRAGMA"),
            (DbType::SQLServer, "NOLOCK"),
            (DbType::ClickHouse, "PREWHERE"),
        ] {
            assert!(
                dialects::completions(db_type)
                    .iter()
                    .any(|item| item.label == expected),
                "missing {expected} for {db_type:?}"
            );
        }
    }
}
