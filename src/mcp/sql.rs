use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::db::DbType;
use crate::db::{SqlDialect, SqlRenderError, SqlRenderResult, SqlScript, TableRef};

pub type SqlBuildResult<T> = SqlRenderResult<T>;
pub type SqlBuildError = SqlRenderError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    View,
    Function,
    Procedure,
    Trigger,
    Database,
}

impl ObjectKind {
    #[cfg(test)]
    fn keyword(self) -> &'static str {
        match self {
            Self::View => "VIEW",
            Self::Function => "FUNCTION",
            Self::Procedure => "PROCEDURE",
            Self::Trigger => "TRIGGER",
            Self::Database => "DATABASE",
        }
    }

    fn operation_name(self, action: &'static str) -> &'static str {
        match (action, self) {
            ("create", Self::View) => "create view",
            ("create", Self::Function) => "create function",
            ("create", Self::Procedure) => "create procedure",
            ("create", Self::Trigger) => "create trigger",
            ("create", Self::Database) => "create database",
            ("drop", Self::View) => "drop view",
            ("drop", Self::Function) => "drop function",
            ("drop", Self::Procedure) => "drop procedure",
            ("drop", Self::Trigger) => "drop trigger",
            ("drop", Self::Database) => "drop database",
            _ => "database object operation",
        }
    }
}

fn unsupported(db_type: &DbType, operation: &'static str) -> SqlBuildError {
    SqlBuildError::Unsupported {
        db_type: *db_type,
        operation,
    }
}

fn ensure_relational(db_type: &DbType, operation: &'static str) -> SqlBuildResult<()> {
    if db_type.capabilities().sql {
        Ok(())
    } else {
        Err(unsupported(db_type, operation))
    }
}

fn ensure_schema_support(db_type: &DbType, operation: &'static str) -> SqlBuildResult<()> {
    if db_type.capabilities().schema_management {
        Ok(())
    } else {
        Err(unsupported(db_type, operation))
    }
}

pub(super) fn ensure_object_operation(
    db_type: &DbType,
    kind: ObjectKind,
    action: &'static str,
) -> SqlBuildResult<()> {
    let capabilities = db_type.capabilities();
    let supported = match kind {
        ObjectKind::View => capabilities.views,
        ObjectKind::Function => capabilities.functions,
        ObjectKind::Procedure => capabilities.procedures,
        ObjectKind::Trigger => capabilities.triggers,
        ObjectKind::Database => capabilities.database_management,
    };

    if supported {
        Ok(())
    } else {
        Err(unsupported(db_type, kind.operation_name(action)))
    }
}

pub fn quote_identifier(db_type: &DbType, identifier: &str) -> SqlBuildResult<String> {
    SqlDialect::new(*db_type).quote_identifier(identifier)
}

pub fn quote_qualified_name(db_type: &DbType, name: &str) -> SqlBuildResult<String> {
    SqlDialect::new(*db_type).quote_qualified_name(name)
}

pub fn build_create_schema(db_type: &DbType, name: &str) -> SqlBuildResult<String> {
    ensure_schema_support(db_type, "create schema")?;
    Ok(format!(
        "CREATE SCHEMA {}",
        quote_identifier(db_type, name)?
    ))
}

pub fn build_drop_schema(db_type: &DbType, name: &str, cascade: bool) -> SqlBuildResult<String> {
    ensure_schema_support(db_type, "drop schema")?;
    let mut sql = format!("DROP SCHEMA {}", quote_identifier(db_type, name)?);
    match (db_type, cascade) {
        (DbType::PostgreSQL, true) => sql.push_str(" CASCADE"),
        (DbType::SQLServer, true) => {
            return Err(SqlBuildError::InvalidInput(
                "SQL Server does not support DROP SCHEMA ... CASCADE".to_string(),
            ));
        }
        // MySQL treats SCHEMA as DATABASE and drops contained objects without
        // a separate CASCADE clause.
        _ => {}
    }
    Ok(sql)
}

/// Validate and return one complete `CREATE TABLE` statement.
///
/// This is deliberately conservative: only one top-level statement is
/// accepted. Semicolons inside quoted strings, identifiers, comments, or
/// PostgreSQL dollar-quoted bodies are ignored.
pub fn build_create_table(db_type: &DbType, sql: &str) -> SqlBuildResult<String> {
    ensure_relational(db_type, "create table")?;
    validate_create_sql_prefixes(
        db_type,
        sql,
        &[
            "CREATE TABLE",
            "CREATE TEMP TABLE",
            "CREATE TEMPORARY TABLE",
        ],
    )
}

pub fn build_drop_table(db_type: &DbType, name: &str) -> SqlBuildResult<String> {
    ensure_relational(db_type, "drop table")?;
    Ok(format!(
        "DROP TABLE {}",
        quote_qualified_name(db_type, name)?
    ))
}

/// Validate a full object-creation statement against the requested object
/// kind. The returned SQL has a single optional trailing semicolon removed.
pub fn build_create_object(
    db_type: &DbType,
    kind: ObjectKind,
    sql: &str,
) -> SqlBuildResult<String> {
    ensure_object_operation(db_type, kind, "create")?;
    let prefix = match kind {
        ObjectKind::View => "CREATE VIEW",
        ObjectKind::Function => "CREATE FUNCTION",
        ObjectKind::Procedure => "CREATE PROCEDURE",
        ObjectKind::Trigger => "CREATE TRIGGER",
        ObjectKind::Database => "CREATE DATABASE",
    };
    validate_create_sql(db_type, sql, prefix)
}

#[cfg(test)]
pub fn build_drop_object(db_type: &DbType, kind: ObjectKind, name: &str) -> SqlBuildResult<String> {
    ensure_object_operation(db_type, kind, "drop")?;

    if kind == ObjectKind::Database {
        return Ok(format!(
            "DROP {} {}",
            kind.keyword(),
            quote_identifier(db_type, name)?
        ));
    }

    if kind == ObjectKind::Trigger && matches!(db_type, DbType::PostgreSQL) {
        let mut parts: Vec<&str> = name.split('.').collect();
        if !(2..=3).contains(&parts.len()) {
            return Err(SqlBuildError::InvalidInput(
                "PostgreSQL trigger drops require 'table.trigger' or 'schema.table.trigger'"
                    .to_string(),
            ));
        }
        let trigger = parts.pop().expect("length checked");
        let table = parts.join(".");
        return Ok(format!(
            "DROP TRIGGER {} ON {}",
            quote_identifier(db_type, trigger)?,
            quote_qualified_name(db_type, &table)?
        ));
    }

    Ok(format!(
        "DROP {} {}",
        kind.keyword(),
        quote_qualified_name(db_type, name)?
    ))
}

pub fn build_insert_row(
    db_type: &DbType,
    table: &str,
    columns: &[String],
    values: &[Value],
) -> SqlBuildResult<String> {
    let table = TableRef::parse(*db_type, table)?;
    build_insert_row_for_table(db_type, &table, columns, values)
}

pub fn build_insert_row_for_table(
    db_type: &DbType,
    table: &TableRef,
    columns: &[String],
    values: &[Value],
) -> SqlBuildResult<String> {
    SqlDialect::new(*db_type).build_insert_row(table, columns, values)
}

pub fn build_update_row(
    db_type: &DbType,
    table: &str,
    primary_key_column: &str,
    primary_key_value: &Value,
    column: &str,
    new_value: &Value,
) -> SqlBuildResult<String> {
    let table = TableRef::parse(*db_type, table)?;
    build_update_row_for_table(
        db_type,
        &table,
        primary_key_column,
        primary_key_value,
        column,
        new_value,
    )
}

pub fn build_update_row_for_table(
    db_type: &DbType,
    table: &TableRef,
    primary_key_column: &str,
    primary_key_value: &Value,
    column: &str,
    new_value: &Value,
) -> SqlBuildResult<String> {
    SqlDialect::new(*db_type).build_update_row(
        table,
        primary_key_column,
        primary_key_value,
        column,
        new_value,
    )
}

pub fn build_delete_rows(
    db_type: &DbType,
    table: &str,
    primary_key_column: &str,
    primary_key_values: &[Value],
) -> SqlBuildResult<String> {
    let table = TableRef::parse(*db_type, table)?;
    build_delete_rows_for_table(db_type, &table, primary_key_column, primary_key_values)
}

pub fn build_delete_rows_for_table(
    db_type: &DbType,
    table: &TableRef,
    primary_key_column: &str,
    primary_key_values: &[Value],
) -> SqlBuildResult<String> {
    SqlDialect::new(*db_type).build_delete_rows(table, primary_key_column, primary_key_values)
}

/// Validate one `CREATE` statement against an exact keyword prefix.
pub fn validate_create_sql(
    db_type: &DbType,
    sql: &str,
    expected_prefix: &str,
) -> SqlBuildResult<String> {
    validate_create_sql_prefixes(db_type, sql, &[expected_prefix])
}

fn validate_create_sql_prefixes(
    db_type: &DbType,
    sql: &str,
    expected_prefixes: &[&str],
) -> SqlBuildResult<String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err(SqlBuildError::InvalidCreateSql(
            "CREATE statement must not be empty".to_string(),
        ));
    }
    if trimmed.chars().any(|character| character == '\0') {
        return Err(SqlBuildError::InvalidCreateSql(
            "CREATE statement must not contain NUL".to_string(),
        ));
    }
    let mut statements = SqlScript::parse(*db_type, trimmed)
        .map_err(|error| SqlBuildError::InvalidCreateSql(error.to_string()))?
        .into_statements();
    if statements.len() != 1 {
        return Err(SqlBuildError::InvalidCreateSql(
            "only one CREATE statement is allowed".to_string(),
        ));
    }
    let statement = statements.pop().expect("length checked");
    if !expected_prefixes
        .iter()
        .any(|prefix| has_keyword_prefix(&statement, prefix))
    {
        return Err(SqlBuildError::InvalidCreateSql(format!(
            "expected statement prefix: {}",
            expected_prefixes.join(" or ")
        )));
    }

    Ok(statement)
}

fn has_keyword_prefix(sql: &str, expected_prefix: &str) -> bool {
    let expected_words: Vec<&str> = expected_prefix.split_whitespace().collect();
    if expected_words.is_empty() {
        return false;
    }

    let mut rest = sql;
    for expected in expected_words {
        rest = rest.trim_start();
        let word_len = rest
            .char_indices()
            .take_while(|(_, character)| character.is_ascii_alphabetic())
            .last()
            .map_or(0, |(index, character)| index + character.len_utf8());
        if word_len == 0 || !rest[..word_len].eq_ignore_ascii_case(expected) {
            return false;
        }
        rest = &rest[word_len..];
    }
    rest.chars()
        .next()
        .is_none_or(|character| !character.is_ascii_alphanumeric() && character != '_')
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn quotes_and_escapes_identifier_delimiters() {
        assert_eq!(
            quote_qualified_name(&DbType::PostgreSQL, "public.weird\"table").unwrap(),
            "\"public\".\"weird\"\"table\""
        );
        assert_eq!(
            quote_qualified_name(&DbType::MySQL, "app.weird`table").unwrap(),
            "`app`.`weird``table`"
        );
        assert_eq!(
            quote_qualified_name(&DbType::SQLServer, "dbo.weird]table").unwrap(),
            "[dbo].[weird]]table]"
        );
        assert_eq!(
            quote_qualified_name(&DbType::ClickHouse, r"app.weird\`table").unwrap(),
            r"`app`.`weird\\\`table`"
        );
    }

    #[test]
    fn rejects_invalid_qualified_names() {
        for name in ["", ".users", "public.", "public..users", " public.users"] {
            assert!(quote_qualified_name(&DbType::PostgreSQL, name).is_err());
        }
        assert_eq!(
            quote_identifier(&DbType::PostgreSQL, "public.users").unwrap(),
            "\"public.users\""
        );
        assert!(quote_qualified_name(&DbType::MySQL, "server.app.users").is_err());
        assert!(quote_qualified_name(&DbType::PostgreSQL, "db.public.users").is_err());
    }

    #[test]
    fn schema_and_drop_builders_are_dialect_aware() {
        assert_eq!(
            build_create_schema(&DbType::PostgreSQL, "audit\"log").unwrap(),
            "CREATE SCHEMA \"audit\"\"log\""
        );
        assert_eq!(
            build_drop_table(&DbType::MySQL, "app.order`items").unwrap(),
            "DROP TABLE `app`.`order``items`"
        );
        assert_eq!(
            build_drop_schema(&DbType::PostgreSQL, "audit", true).unwrap(),
            "DROP SCHEMA \"audit\" CASCADE"
        );
        assert!(build_drop_schema(&DbType::SQLServer, "audit", true).is_err());
        assert!(build_create_schema(&DbType::SQLite, "main").is_err());
        assert!(build_drop_table(&DbType::MongoDB, "users").is_err());
    }

    #[test]
    fn object_operations_follow_engine_capabilities() {
        assert!(ensure_object_operation(&DbType::SQLite, ObjectKind::View, "drop").is_ok());
        assert!(ensure_object_operation(&DbType::SQLite, ObjectKind::Function, "drop").is_err());
        assert!(ensure_object_operation(&DbType::ClickHouse, ObjectKind::Database, "drop").is_ok());
        assert!(
            ensure_object_operation(&DbType::ClickHouse, ObjectKind::Procedure, "drop").is_err()
        );
        assert!(ensure_object_operation(&DbType::MongoDB, ObjectKind::View, "drop").is_err());
    }

    #[test]
    fn validates_one_create_statement_and_allows_quoted_semicolons() {
        assert_eq!(
            build_create_table(
                &DbType::PostgreSQL,
                " CREATE TABLE \"events\" (\"text\" text DEFAULT ';'); ",
            )
            .unwrap(),
            "CREATE TABLE \"events\" (\"text\" text DEFAULT ';')"
        );
        assert!(build_create_table(
            &DbType::PostgreSQL,
            "CREATE TABLE events(id int); DROP TABLE users",
        )
        .is_err());
        assert!(build_create_table(&DbType::PostgreSQL, "DROP TABLE events").is_err());
    }

    #[test]
    fn validates_postgres_dollar_quoted_function_body() {
        let sql = "CREATE FUNCTION public.f() RETURNS void LANGUAGE plpgsql \
                   AS $$ BEGIN PERFORM 1; END; $$;";
        assert_eq!(
            build_create_object(&DbType::PostgreSQL, ObjectKind::Function, sql).unwrap(),
            sql.trim_end_matches(';')
        );
        assert!(build_create_object(
            &DbType::PostgreSQL,
            ObjectKind::Function,
            "CREATE OR REPLACE FUNCTION public.f() RETURNS void LANGUAGE sql AS $$ SELECT 1 $$",
        )
        .is_err());
    }

    #[test]
    fn postgres_trigger_drop_requires_parent_table() {
        assert_eq!(
            build_drop_object(
                &DbType::PostgreSQL,
                ObjectKind::Trigger,
                "audit.events.capture_change",
            )
            .unwrap(),
            "DROP TRIGGER \"capture_change\" ON \"audit\".\"events\""
        );
        assert!(
            build_drop_object(&DbType::PostgreSQL, ObjectKind::Trigger, "capture_change",).is_err()
        );
    }

    #[test]
    fn builds_insert_with_safe_literals() {
        let columns = vec!["name".to_string(), "enabled".to_string()];
        let values = vec![json!("O'Reilly\\admin"), json!(true)];
        assert_eq!(
            build_insert_row(&DbType::PostgreSQL, "public.users", &columns, &values).unwrap(),
            "INSERT INTO \"public\".\"users\" (\"name\", \"enabled\") \
             VALUES (E'O''Reilly\\\\admin', TRUE)"
        );
    }

    #[test]
    fn builds_update_and_delete_by_non_null_primary_key() {
        assert_eq!(
            build_update_row(
                &DbType::SQLServer,
                "dbo.users",
                "id",
                &json!(7),
                "display_name",
                &json!("Alice"),
            )
            .unwrap(),
            "UPDATE [dbo].[users] SET [display_name] = N'Alice' WHERE [id] = 7"
        );
        assert_eq!(
            build_delete_rows(&DbType::SQLite, "users", "id", &[json!(7), json!(9)],).unwrap(),
            "DELETE FROM \"users\" WHERE \"id\" IN (7, 9)"
        );
        assert!(build_delete_rows(&DbType::SQLite, "users", "id", &[]).is_err());
        assert!(build_update_row(
            &DbType::SQLite,
            "users",
            "id",
            &Value::Null,
            "name",
            &json!("x"),
        )
        .is_err());
    }

    #[test]
    fn builds_clickhouse_mutations_with_clickhouse_literals() {
        assert_eq!(
            build_update_row(
                &DbType::ClickHouse,
                "analytics.events",
                "id",
                &json!(7),
                "message",
                &json!("it's\\ready"),
            )
            .unwrap(),
            "ALTER TABLE `analytics.events` UPDATE `message` = 'it\\'s\\\\ready' WHERE `id` = 7 SETTINGS mutations_sync = 1"
        );
        assert_eq!(
            build_delete_rows(
                &DbType::ClickHouse,
                "analytics.events",
                "id",
                &[json!(7), json!(9)],
            )
            .unwrap(),
            "ALTER TABLE `analytics.events` DELETE WHERE `id` IN (7, 9) SETTINGS mutations_sync = 1"
        );
        assert!(build_create_schema(&DbType::ClickHouse, "analytics").is_err());
    }

    #[test]
    fn rejects_structured_rows_for_document_and_key_value_databases() {
        let columns = vec!["name".to_string()];
        let values = vec![json!("Alice")];
        assert!(build_insert_row(&DbType::MongoDB, "users", &columns, &values).is_err());
        assert!(build_delete_rows(&DbType::Redis, "keys", "key", &values).is_err());
    }

    #[test]
    fn rejects_insert_shape_mismatch_and_nul_values() {
        let columns = vec!["name".to_string()];
        assert!(build_insert_row(&DbType::SQLite, "users", &columns, &[]).is_err());
        assert!(build_insert_row(&DbType::SQLite, "users", &columns, &[json!("a\0b")],).is_err());
    }
}
