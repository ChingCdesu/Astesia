use std::{error::Error, fmt};

use serde_json::Value;

use crate::db::{DbType, SqlDialect, TableRef};

use super::connections::ConnectionManager;
use super::QueryTarget;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DatabaseObjectKind {
    Database,
    Schema,
    Table,
    View,
    Function,
    Procedure,
    Trigger,
    User,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TriggerTiming {
    Before,
    After,
    InsteadOf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TriggerEvent {
    Insert,
    Update,
    Delete,
    Truncate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TableColumnSpec {
    pub(crate) name: String,
    pub(crate) data_type: String,
    pub(crate) nullable: bool,
    pub(crate) primary_key: bool,
    pub(crate) default_value: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CreateObjectSpec {
    Database {
        name: String,
    },
    Schema {
        name: String,
    },
    Table {
        name: String,
        columns: Vec<TableColumnSpec>,
    },
    View {
        name: String,
        query: String,
    },
    Function {
        name: String,
        arguments: String,
        return_type: String,
        language: String,
        body: String,
    },
    Procedure {
        name: String,
        arguments: String,
        language: String,
        body: String,
    },
    Trigger {
        name: String,
        table: TableRef,
        timing: TriggerTiming,
        event: TriggerEvent,
        body: String,
    },
    User {
        name: String,
        host: Option<String>,
        password: String,
    },
}

impl CreateObjectSpec {
    pub(crate) const fn kind(&self) -> DatabaseObjectKind {
        match self {
            Self::Database { .. } => DatabaseObjectKind::Database,
            Self::Schema { .. } => DatabaseObjectKind::Schema,
            Self::Table { .. } => DatabaseObjectKind::Table,
            Self::View { .. } => DatabaseObjectKind::View,
            Self::Function { .. } => DatabaseObjectKind::Function,
            Self::Procedure { .. } => DatabaseObjectKind::Procedure,
            Self::Trigger { .. } => DatabaseObjectKind::Trigger,
            Self::User { .. } => DatabaseObjectKind::User,
        }
    }

    fn display_identity(&self) -> String {
        match self {
            Self::Database { name }
            | Self::Schema { name }
            | Self::Table { name, .. }
            | Self::View { name, .. }
            | Self::Function { name, .. }
            | Self::Procedure { name, .. } => name.clone(),
            Self::Trigger { name, table, .. } => format!("{name} ON {table}"),
            Self::User { name, host, .. } => host
                .as_deref()
                .map_or_else(|| name.clone(), |host| format!("{name}@{host}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DropObjectTarget {
    Database(String),
    Schema(String),
    Table(String),
    View(String),
    Function(String),
    Procedure(String),
    Trigger { name: String, table: String },
    User { name: String, host: Option<String> },
}

impl DropObjectTarget {
    pub(crate) const fn kind(&self) -> DatabaseObjectKind {
        match self {
            Self::Database(_) => DatabaseObjectKind::Database,
            Self::Schema(_) => DatabaseObjectKind::Schema,
            Self::Table(_) => DatabaseObjectKind::Table,
            Self::View(_) => DatabaseObjectKind::View,
            Self::Function(_) => DatabaseObjectKind::Function,
            Self::Procedure(_) => DatabaseObjectKind::Procedure,
            Self::Trigger { .. } => DatabaseObjectKind::Trigger,
            Self::User { .. } => DatabaseObjectKind::User,
        }
    }

    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Database(name)
            | Self::Schema(name)
            | Self::Table(name)
            | Self::View(name)
            | Self::Function(name)
            | Self::Procedure(name)
            | Self::Trigger { name, .. }
            | Self::User { name, .. } => name,
        }
    }

    fn display_identity(&self) -> String {
        match self {
            Self::Trigger { name, table } => format!("{name} ON {table}"),
            Self::User {
                name,
                host: Some(host),
            } => format!("{name}@{host}"),
            target => target.name().to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ObjectMutation {
    Create(CreateObjectSpec),
    Rename {
        kind: DatabaseObjectKind,
        name: String,
        new_name: String,
    },
    Drop(DropObjectTarget),
}

impl ObjectMutation {
    pub(crate) const fn kind(&self) -> DatabaseObjectKind {
        match self {
            Self::Create(spec) => spec.kind(),
            Self::Rename { kind, .. } => *kind,
            Self::Drop(target) => target.kind(),
        }
    }

    pub(crate) fn display_identity(&self) -> String {
        match self {
            Self::Create(spec) => spec.display_identity(),
            Self::Rename { name, new_name, .. } => format!("{name} → {new_name}"),
            Self::Drop(target) => target.display_identity(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ObjectMutationError {
    Connection(String),
    SessionChanged { expected: u64, actual: u64 },
    EngineChanged { expected: DbType, actual: DbType },
    Unsupported { db_type: DbType, operation: String },
    Invalid(String),
    Execution(String),
}

impl fmt::Display for ObjectMutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connection(message) | Self::Invalid(message) | Self::Execution(message) => {
                formatter.write_str(message)
            }
            Self::SessionChanged { expected, actual } => write!(
                formatter,
                "Connection session changed before the object mutation (expected {expected}, found {actual})"
            ),
            Self::EngineChanged { expected, actual } => write!(
                formatter,
                "Connection engine changed before the object mutation (expected {expected:?}, found {actual:?})"
            ),
            Self::Unsupported { db_type, operation } => {
                write!(formatter, "{operation} is not supported for {db_type:?}")
            }
        }
    }
}

impl Error for ObjectMutationError {}

#[derive(Clone)]
pub(crate) struct ObjectService {
    manager: ConnectionManager,
}

impl ObjectService {
    pub(super) fn new(manager: ConnectionManager) -> Self {
        Self { manager }
    }

    pub(crate) async fn execute(
        &self,
        target: &QueryTarget,
        mutation: &ObjectMutation,
    ) -> Result<(), ObjectMutationError> {
        let (handle, actual_generation) = self
            .manager
            .driver_session(&target.connection_id)
            .await
            .map_err(ObjectMutationError::Connection)?;
        if actual_generation != target.session_generation {
            return Err(ObjectMutationError::SessionChanged {
                expected: target.session_generation,
                actual: actual_generation,
            });
        }
        let driver = handle
            .lock_active()
            .await
            .map_err(ObjectMutationError::Connection)?;
        let actual_db_type = driver.db_type();
        if actual_db_type != target.db_type {
            return Err(ObjectMutationError::EngineChanged {
                expected: target.db_type,
                actual: actual_db_type,
            });
        }
        if matches!(
            mutation,
            ObjectMutation::Drop(DropObjectTarget::Database(name)) if name == &target.database
        ) {
            return Err(ObjectMutationError::Invalid(
                "Switch to another database before dropping the active database".to_string(),
            ));
        }
        let sql = render_object_mutation(actual_db_type, mutation)?;
        driver
            .execute_query(&target.database, &sql)
            .await
            .map_err(|error| {
                let message = if matches!(
                    mutation,
                    ObjectMutation::Create(CreateObjectSpec::User { .. })
                ) {
                    "The database rejected the user operation".to_string()
                } else {
                    error.to_string()
                };
                ObjectMutationError::Execution(message)
            })?;
        Ok(())
    }
}

pub(crate) fn render_object_mutation(
    db_type: DbType,
    mutation: &ObjectMutation,
) -> Result<String, ObjectMutationError> {
    match mutation {
        ObjectMutation::Create(spec) => render_create(db_type, spec),
        ObjectMutation::Rename {
            kind,
            name,
            new_name,
        } => render_rename(db_type, *kind, name, new_name),
        ObjectMutation::Drop(target) => render_drop(db_type, target),
    }
}

fn render_create(db_type: DbType, spec: &CreateObjectSpec) -> Result<String, ObjectMutationError> {
    require_create_support(db_type, spec.kind())?;
    let dialect = SqlDialect::new(db_type);
    match spec {
        CreateObjectSpec::Database { name } => Ok(format!(
            "CREATE DATABASE {}",
            quote_identifier(dialect, name)?
        )),
        CreateObjectSpec::Schema { name } => Ok(format!(
            "CREATE SCHEMA {}",
            quote_identifier(dialect, name)?
        )),
        CreateObjectSpec::Table { name, columns } => {
            if columns.is_empty() {
                return Err(ObjectMutationError::Invalid(
                    "A table requires at least one column".to_string(),
                ));
            }
            let mut primary_keys = Vec::new();
            let mut definitions = Vec::with_capacity(columns.len());
            for column in columns {
                let column_name = quote_identifier(dialect, &column.name)?;
                let data_type = validate_fragment(&column.data_type, "column type")?;
                let data_type =
                    if db_type == DbType::ClickHouse && column.nullable && !column.primary_key {
                        clickhouse_nullable_type(data_type)?
                    } else {
                        data_type.to_string()
                    };
                let mut definition = format!("{column_name} {data_type}");
                if column.primary_key {
                    primary_keys.push(column_name.clone());
                }
                if db_type != DbType::ClickHouse && !column.nullable && !column.primary_key {
                    definition.push_str(" NOT NULL");
                }
                if let Some(default_value) = column.default_value.as_deref() {
                    definition.push_str(" DEFAULT ");
                    definition.push_str(validate_fragment(default_value, "column default")?);
                }
                definitions.push(definition);
            }
            if db_type != DbType::ClickHouse && !primary_keys.is_empty() {
                definitions.push(format!("PRIMARY KEY ({})", primary_keys.join(", ")));
            }
            let mut sql = format!(
                "CREATE TABLE {} (\n  {}\n)",
                quote_name(dialect, name)?,
                definitions.join(",\n  ")
            );
            if db_type == DbType::ClickHouse {
                sql.push_str("\nENGINE = MergeTree\nORDER BY ");
                if primary_keys.is_empty() {
                    sql.push_str("tuple()");
                } else {
                    sql.push_str(&format!("({})", primary_keys.join(", ")));
                }
            }
            Ok(sql)
        }
        CreateObjectSpec::View { name, query } => {
            let query = require_body(query, "view query")?;
            Ok(format!(
                "CREATE VIEW {} AS\n{query}",
                quote_name(dialect, name)?
            ))
        }
        CreateObjectSpec::Function {
            name,
            arguments,
            return_type,
            language,
            body,
        } => render_create_function(
            db_type,
            dialect,
            name,
            arguments,
            return_type,
            language,
            body,
        ),
        CreateObjectSpec::Procedure {
            name,
            arguments,
            language,
            body,
        } => render_create_procedure(db_type, dialect, name, arguments, language, body),
        CreateObjectSpec::Trigger {
            name,
            table,
            timing,
            event,
            body,
        } => render_create_trigger(db_type, dialect, name, table, *timing, *event, body),
        CreateObjectSpec::User {
            name,
            host,
            password,
        } => render_create_user(db_type, dialect, name, host.as_deref(), password),
    }
}

fn render_create_function(
    db_type: DbType,
    dialect: SqlDialect,
    name: &str,
    arguments: &str,
    return_type: &str,
    language: &str,
    body: &str,
) -> Result<String, ObjectMutationError> {
    let name = quote_name(dialect, name)?;
    let arguments = validate_fragment_allow_empty(arguments, "function arguments")?;
    let body = require_body(body, "function body")?;
    match db_type {
        DbType::PostgreSQL => Ok(format!(
            "CREATE FUNCTION {name}({arguments})\nRETURNS {}\nLANGUAGE {}\nAS {}",
            validate_fragment(return_type, "return type")?,
            quote_identifier(dialect, language)?,
            dollar_quote(body)
        )),
        DbType::MySQL => Ok(format!(
            "CREATE FUNCTION {name}({arguments})\nRETURNS {}\nDETERMINISTIC\nBEGIN\n{body}\nEND",
            validate_fragment(return_type, "return type")?
        )),
        DbType::SQLServer => Ok(format!(
            "CREATE FUNCTION {name}({arguments})\nRETURNS {}\nAS\nBEGIN\n{body}\nEND",
            validate_fragment(return_type, "return type")?
        )),
        DbType::ClickHouse => Ok(format!("CREATE FUNCTION {name} AS {body}")),
        _ => unsupported(db_type, "create function"),
    }
}

fn render_create_procedure(
    db_type: DbType,
    dialect: SqlDialect,
    name: &str,
    arguments: &str,
    language: &str,
    body: &str,
) -> Result<String, ObjectMutationError> {
    let name = quote_name(dialect, name)?;
    let arguments = validate_fragment_allow_empty(arguments, "procedure arguments")?;
    let body = require_body(body, "procedure body")?;
    match db_type {
        DbType::PostgreSQL => Ok(format!(
            "CREATE PROCEDURE {name}({arguments})\nLANGUAGE {}\nAS {}",
            quote_identifier(dialect, language)?,
            dollar_quote(body)
        )),
        DbType::MySQL => Ok(format!(
            "CREATE PROCEDURE {name}({arguments})\nBEGIN\n{body}\nEND"
        )),
        DbType::SQLServer => {
            let arguments = if arguments.is_empty() {
                String::new()
            } else {
                format!(" {arguments}")
            };
            Ok(format!(
                "CREATE PROCEDURE {name}{arguments}\nAS\nBEGIN\n{body}\nEND"
            ))
        }
        _ => unsupported(db_type, "create procedure"),
    }
}

fn render_create_trigger(
    db_type: DbType,
    dialect: SqlDialect,
    name: &str,
    table: &TableRef,
    timing: TriggerTiming,
    event: TriggerEvent,
    body: &str,
) -> Result<String, ObjectMutationError> {
    let body = require_body(body, "trigger body")?;
    let name = if db_type == DbType::PostgreSQL {
        quote_identifier(dialect, last_name_part(name))?
    } else {
        quote_name(dialect, name)?
    };
    let table = dialect
        .quote_table_ref(table)
        .map_err(|error| ObjectMutationError::Invalid(error.to_string()))?;
    let timing = trigger_timing(db_type, timing)?;
    let event = trigger_event(db_type, event)?;
    match db_type {
        DbType::PostgreSQL => Ok(format!(
            "CREATE TRIGGER {name}\n{timing} {event} ON {table}\nFOR EACH ROW\nEXECUTE FUNCTION {}",
            validate_fragment(body, "trigger function")?
        )),
        DbType::MySQL => Ok(format!(
            "CREATE TRIGGER {name}\n{timing} {event} ON {table}\nFOR EACH ROW\nBEGIN\n{body}\nEND"
        )),
        DbType::SQLite => Ok(format!(
            "CREATE TRIGGER {name}\n{timing} {event} ON {table}\nBEGIN\n{body}\nEND"
        )),
        DbType::SQLServer => Ok(format!(
            "CREATE TRIGGER {name} ON {table}\n{timing} {event}\nAS\nBEGIN\n{body}\nEND"
        )),
        _ => unsupported(db_type, "create trigger"),
    }
}

fn render_create_user(
    db_type: DbType,
    dialect: SqlDialect,
    name: &str,
    host: Option<&str>,
    password: &str,
) -> Result<String, ObjectMutationError> {
    if password.is_empty() {
        return Err(ObjectMutationError::Invalid(
            "A password is required".to_string(),
        ));
    }
    let password = string_literal(dialect, password)?;
    match db_type {
        DbType::PostgreSQL => Ok(format!(
            "CREATE USER {} WITH PASSWORD {password}",
            quote_identifier(dialect, name)?
        )),
        DbType::MySQL => Ok(format!(
            "CREATE USER {}@{} IDENTIFIED BY {password}",
            string_literal(dialect, name)?,
            string_literal(dialect, host.unwrap_or("%"))?
        )),
        DbType::SQLServer => Ok(format!(
            "CREATE LOGIN {} WITH PASSWORD = {password}",
            quote_identifier(dialect, name)?
        )),
        DbType::ClickHouse => Ok(format!(
            "CREATE USER {} IDENTIFIED WITH sha256_password BY {password}",
            quote_identifier(dialect, name)?
        )),
        _ => unsupported(db_type, "create user"),
    }
}

fn render_rename(
    db_type: DbType,
    kind: DatabaseObjectKind,
    name: &str,
    new_name: &str,
) -> Result<String, ObjectMutationError> {
    let dialect = SqlDialect::new(db_type);
    let quoted_new_name = quote_identifier(dialect, new_name)?;
    match (db_type, kind) {
        (DbType::PostgreSQL, DatabaseObjectKind::Database) => Ok(format!(
            "ALTER DATABASE {} RENAME TO {quoted_new_name}",
            quote_identifier(dialect, name)?
        )),
        (DbType::SQLServer, DatabaseObjectKind::Database) => Ok(format!(
            "ALTER DATABASE {} MODIFY NAME = {quoted_new_name}",
            quote_identifier(dialect, name)?
        )),
        (DbType::ClickHouse, DatabaseObjectKind::Database) => Ok(format!(
            "RENAME DATABASE {} TO {quoted_new_name}",
            quote_identifier(dialect, name)?
        )),
        (DbType::PostgreSQL, DatabaseObjectKind::Schema) => Ok(format!(
            "ALTER SCHEMA {} RENAME TO {quoted_new_name}",
            quote_identifier(dialect, name)?
        )),
        (DbType::MySQL | DbType::ClickHouse, DatabaseObjectKind::Table) => Ok(format!(
            "RENAME TABLE {} TO {quoted_new_name}",
            quote_name(dialect, name)?
        )),
        (DbType::PostgreSQL | DbType::SQLite, DatabaseObjectKind::Table) => Ok(format!(
            "ALTER TABLE {} RENAME TO {quoted_new_name}",
            quote_name(dialect, name)?
        )),
        (DbType::SQLServer, DatabaseObjectKind::Table) => {
            let old_name = quote_name(dialect, name)?;
            Ok(format!(
                "EXEC sp_rename {}, {}, 'OBJECT'",
                string_literal(dialect, &old_name)?,
                string_literal(dialect, new_name.trim())?
            ))
        }
        _ => unsupported(db_type, &format!("rename {kind:?}")),
    }
}

fn render_drop(db_type: DbType, target: &DropObjectTarget) -> Result<String, ObjectMutationError> {
    let kind = target.kind();
    let name = target.name();
    require_drop_support(db_type, kind)?;
    let dialect = SqlDialect::new(db_type);
    if let (DbType::PostgreSQL, DropObjectTarget::Trigger { table, .. }) = (db_type, target) {
        let table = TableRef::parse(db_type, table)
            .map_err(|error| ObjectMutationError::Invalid(error.to_string()))?;
        return Ok(format!(
            "DROP TRIGGER {} ON {}",
            quote_identifier(dialect, last_name_part(name))?,
            dialect
                .quote_table_ref(&table)
                .map_err(|error| ObjectMutationError::Invalid(error.to_string()))?
        ));
    }
    if let (DbType::MySQL, DropObjectTarget::User { host, .. }) = (db_type, target) {
        return Ok(format!(
            "DROP USER {}@{}",
            string_literal(dialect, name)?,
            string_literal(dialect, host.as_deref().unwrap_or("%"))?
        ));
    }
    let keyword = match kind {
        DatabaseObjectKind::Database => "DATABASE",
        DatabaseObjectKind::Schema => "SCHEMA",
        DatabaseObjectKind::Table => "TABLE",
        DatabaseObjectKind::View => "VIEW",
        DatabaseObjectKind::Function => "FUNCTION",
        DatabaseObjectKind::Procedure => "PROCEDURE",
        DatabaseObjectKind::Trigger => "TRIGGER",
        DatabaseObjectKind::User if db_type == DbType::SQLServer => "LOGIN",
        DatabaseObjectKind::User => "USER",
    };
    let name = if matches!(
        kind,
        DatabaseObjectKind::Database | DatabaseObjectKind::Schema | DatabaseObjectKind::User
    ) {
        quote_identifier(dialect, name)?
    } else if matches!(
        kind,
        DatabaseObjectKind::Function | DatabaseObjectKind::Procedure
    ) && db_type == DbType::PostgreSQL
    {
        quote_postgres_routine_identity(dialect, name)?
    } else {
        quote_name(dialect, name)?
    };
    let cascade = if db_type == DbType::PostgreSQL && kind == DatabaseObjectKind::Schema {
        " CASCADE"
    } else {
        ""
    };
    Ok(format!("DROP {keyword} {name}{cascade}"))
}

fn require_create_support(
    db_type: DbType,
    kind: DatabaseObjectKind,
) -> Result<(), ObjectMutationError> {
    if object_kind_can_create(db_type, kind) {
        Ok(())
    } else {
        unsupported(db_type, &format!("create {kind:?}"))
    }
}

pub(crate) fn object_kind_can_create(db_type: DbType, kind: DatabaseObjectKind) -> bool {
    let capabilities = db_type.capabilities();
    match kind {
        DatabaseObjectKind::Database => capabilities.database_management,
        DatabaseObjectKind::Schema => capabilities.schema_management,
        DatabaseObjectKind::Table => capabilities.sql,
        DatabaseObjectKind::View => capabilities.views,
        DatabaseObjectKind::Function => capabilities.functions,
        DatabaseObjectKind::Procedure => capabilities.procedures,
        DatabaseObjectKind::Trigger => capabilities.triggers,
        DatabaseObjectKind::User => capabilities.users,
    }
}

pub(crate) fn object_kind_can_rename(db_type: DbType, kind: DatabaseObjectKind) -> bool {
    matches!(
        (db_type, kind),
        (
            DbType::PostgreSQL | DbType::SQLServer | DbType::ClickHouse,
            DatabaseObjectKind::Database
        ) | (DbType::PostgreSQL, DatabaseObjectKind::Schema)
            | (
                DbType::MySQL
                    | DbType::PostgreSQL
                    | DbType::SQLite
                    | DbType::SQLServer
                    | DbType::ClickHouse,
                DatabaseObjectKind::Table
            )
    )
}

pub(crate) fn object_kind_can_drop(db_type: DbType, kind: DatabaseObjectKind) -> bool {
    object_kind_can_create(db_type, kind)
}

fn require_drop_support(
    db_type: DbType,
    kind: DatabaseObjectKind,
) -> Result<(), ObjectMutationError> {
    if object_kind_can_drop(db_type, kind) {
        Ok(())
    } else {
        unsupported(db_type, &format!("drop {kind:?}"))
    }
}

fn unsupported<T>(db_type: DbType, operation: &str) -> Result<T, ObjectMutationError> {
    Err(ObjectMutationError::Unsupported {
        db_type,
        operation: operation.to_string(),
    })
}

fn quote_identifier(dialect: SqlDialect, identifier: &str) -> Result<String, ObjectMutationError> {
    dialect
        .quote_identifier(identifier.trim())
        .map_err(|error| ObjectMutationError::Invalid(error.to_string()))
}

fn quote_name(dialect: SqlDialect, name: &str) -> Result<String, ObjectMutationError> {
    dialect
        .quote_qualified_name(name.trim())
        .map_err(|error| ObjectMutationError::Invalid(error.to_string()))
}

fn string_literal(dialect: SqlDialect, value: &str) -> Result<String, ObjectMutationError> {
    dialect
        .literal(&Value::String(value.to_string()))
        .map_err(|error| ObjectMutationError::Invalid(error.to_string()))
}

fn validate_fragment<'a>(value: &'a str, label: &str) -> Result<&'a str, ObjectMutationError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ObjectMutationError::Invalid(format!(
            "{label} must not be empty"
        )));
    }
    validate_fragment_allow_empty(value, label)
}

fn validate_fragment_allow_empty<'a>(
    value: &'a str,
    label: &str,
) -> Result<&'a str, ObjectMutationError> {
    let value = value.trim();
    if value.contains(';') || value.contains("--") || value.contains("/*") {
        return Err(ObjectMutationError::Invalid(format!(
            "{label} must be one SQL fragment"
        )));
    }
    Ok(value)
}

fn require_body<'a>(body: &'a str, label: &str) -> Result<&'a str, ObjectMutationError> {
    let body = body.trim();
    if body.is_empty() {
        Err(ObjectMutationError::Invalid(format!(
            "{label} must not be empty"
        )))
    } else {
        Ok(body)
    }
}

fn dollar_quote(body: &str) -> String {
    let mut suffix = String::new();
    loop {
        let tag = format!("$astesia{suffix}$");
        if !body.contains(&tag) {
            return format!("{tag}\n{body}\n{tag}");
        }
        suffix.push('_');
    }
}

fn clickhouse_nullable_type(data_type: &str) -> Result<String, ObjectMutationError> {
    let data_type = data_type.trim();
    if data_type.starts_with("Nullable(") {
        return Ok(data_type.to_string());
    }
    if data_type.starts_with("Array(")
        || data_type.starts_with("Map(")
        || data_type.starts_with("Tuple(")
        || data_type.starts_with("Nested(")
    {
        return Err(ObjectMutationError::Invalid(format!(
            "ClickHouse type {data_type} cannot be nullable"
        )));
    }
    if let Some(inner) = data_type
        .strip_prefix("LowCardinality(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return Ok(format!("LowCardinality(Nullable({}))", inner.trim()));
    }
    Ok(format!("Nullable({data_type})"))
}

fn trigger_timing(
    db_type: DbType,
    timing: TriggerTiming,
) -> Result<&'static str, ObjectMutationError> {
    match (db_type, timing) {
        (DbType::PostgreSQL | DbType::MySQL | DbType::SQLite, TriggerTiming::Before) => {
            Ok("BEFORE")
        }
        (_, TriggerTiming::After) => Ok("AFTER"),
        (DbType::PostgreSQL | DbType::SQLite | DbType::SQLServer, TriggerTiming::InsteadOf) => {
            Ok("INSTEAD OF")
        }
        _ => unsupported(db_type, "selected trigger timing"),
    }
}

fn trigger_event(
    db_type: DbType,
    event: TriggerEvent,
) -> Result<&'static str, ObjectMutationError> {
    match event {
        TriggerEvent::Insert => Ok("INSERT"),
        TriggerEvent::Update => Ok("UPDATE"),
        TriggerEvent::Delete => Ok("DELETE"),
        TriggerEvent::Truncate if db_type == DbType::PostgreSQL => Ok("TRUNCATE"),
        TriggerEvent::Truncate => unsupported(db_type, "TRUNCATE trigger"),
    }
}

fn quote_postgres_routine_identity(
    dialect: SqlDialect,
    identity: &str,
) -> Result<String, ObjectMutationError> {
    let open = identity.find('(').ok_or_else(|| {
        ObjectMutationError::Invalid(
            "PostgreSQL routine identity must include argument types".to_string(),
        )
    })?;
    let (name, arguments) = identity.split_at(open);
    if !arguments.ends_with(')') {
        return Err(ObjectMutationError::Invalid(
            "PostgreSQL routine identity has an invalid signature".to_string(),
        ));
    }
    validate_fragment_allow_empty(&arguments[1..arguments.len() - 1], "routine signature")?;
    Ok(format!("{}{arguments}", quote_name(dialect, name)?))
}

fn last_name_part(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name).trim()
}

#[cfg(test)]
mod tests;
