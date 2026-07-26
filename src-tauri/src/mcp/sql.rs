use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::db::DbType;

pub type SqlBuildResult<T> = Result<T, SqlBuildError>;

#[derive(Debug, Clone, PartialEq)]
pub enum SqlBuildError {
    Unsupported {
        db_type: DbType,
        operation: &'static str,
    },
    InvalidIdentifier(String),
    InvalidInput(String),
    InvalidCreateSql(String),
}

impl fmt::Display for SqlBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { db_type, operation } => {
                write!(f, "{operation} is not supported for {db_type:?}")
            }
            Self::InvalidIdentifier(message)
            | Self::InvalidInput(message)
            | Self::InvalidCreateSql(message) => f.write_str(message),
        }
    }
}

impl Error for SqlBuildError {}

impl From<SqlBuildError> for String {
    fn from(error: SqlBuildError) -> Self {
        error.to_string()
    }
}

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
        db_type: db_type.clone(),
        operation,
    }
}

fn ensure_relational(db_type: &DbType, operation: &'static str) -> SqlBuildResult<()> {
    match db_type {
        DbType::MySQL
        | DbType::PostgreSQL
        | DbType::SQLite
        | DbType::SQLServer
        | DbType::ClickHouse => Ok(()),
        DbType::MongoDB | DbType::Redis => Err(unsupported(db_type, operation)),
    }
}

fn ensure_schema_support(db_type: &DbType, operation: &'static str) -> SqlBuildResult<()> {
    match db_type {
        DbType::MySQL | DbType::PostgreSQL | DbType::SQLServer => Ok(()),
        DbType::SQLite | DbType::MongoDB | DbType::Redis | DbType::ClickHouse => {
            Err(unsupported(db_type, operation))
        }
    }
}

fn ensure_object_support(
    db_type: &DbType,
    kind: ObjectKind,
    action: &'static str,
) -> SqlBuildResult<()> {
    let supported = matches!(
        (db_type, kind),
        (DbType::MySQL | DbType::PostgreSQL | DbType::SQLServer, _)
            | (DbType::SQLite, ObjectKind::View | ObjectKind::Trigger)
            | (
                DbType::ClickHouse,
                ObjectKind::View | ObjectKind::Function | ObjectKind::Database
            )
    );

    if supported {
        Ok(())
    } else {
        Err(unsupported(db_type, kind.operation_name(action)))
    }
}

fn validate_identifier_part(identifier: &str) -> SqlBuildResult<()> {
    if identifier.is_empty() {
        return Err(SqlBuildError::InvalidIdentifier(
            "identifier must not be empty".to_string(),
        ));
    }
    if identifier.trim() != identifier {
        return Err(SqlBuildError::InvalidIdentifier(format!(
            "identifier must not have leading or trailing whitespace: {identifier:?}"
        )));
    }
    if identifier.chars().count() > 128 {
        return Err(SqlBuildError::InvalidIdentifier(
            "identifier components must not exceed 128 characters".to_string(),
        ));
    }
    if identifier.chars().any(char::is_control) {
        return Err(SqlBuildError::InvalidIdentifier(
            "identifier must not contain control characters".to_string(),
        ));
    }
    if identifier == "*" {
        return Err(SqlBuildError::InvalidIdentifier(
            "wildcards are not valid object identifiers".to_string(),
        ));
    }
    Ok(())
}

fn qualified_parts(name: &str) -> SqlBuildResult<Vec<&str>> {
    if name.is_empty() {
        return Err(SqlBuildError::InvalidIdentifier(
            "qualified name must not be empty".to_string(),
        ));
    }

    let parts: Vec<&str> = name.split('.').collect();
    if parts.len() > 4 {
        return Err(SqlBuildError::InvalidIdentifier(
            "qualified name must have at most four components".to_string(),
        ));
    }
    for part in &parts {
        validate_identifier_part(part)?;
    }
    Ok(parts)
}

/// Quote one identifier component and escape the dialect's closing delimiter.
///
/// Dotted names are intentionally rejected here. Use [`quote_qualified_name`]
/// when a database/schema-qualified object name is expected.
pub fn quote_identifier(db_type: &DbType, identifier: &str) -> SqlBuildResult<String> {
    ensure_relational(db_type, "quote SQL identifier")?;
    validate_identifier_part(identifier)?;
    if identifier.contains('.') {
        return Err(SqlBuildError::InvalidIdentifier(
            "identifier component must not contain '.', use quote_qualified_name".to_string(),
        ));
    }

    Ok(match db_type {
        DbType::MySQL => {
            format!("`{}`", identifier.replace('`', "``"))
        }
        DbType::ClickHouse => {
            let escaped = identifier.replace('\\', "\\\\").replace('`', "\\`");
            format!("`{escaped}`")
        }
        DbType::PostgreSQL | DbType::SQLite => {
            format!("\"{}\"", identifier.replace('"', "\"\""))
        }
        DbType::SQLServer => format!("[{}]", identifier.replace(']', "]]")),
        DbType::MongoDB | DbType::Redis => unreachable!("relational databases checked above"),
    })
}

/// Quote every component of an unquoted dotted name independently.
pub fn quote_qualified_name(db_type: &DbType, name: &str) -> SqlBuildResult<String> {
    ensure_relational(db_type, "quote SQL qualified name")?;
    let parts = qualified_parts(name)?;
    let max_parts = match db_type {
        DbType::MySQL | DbType::PostgreSQL | DbType::SQLite | DbType::ClickHouse => 2,
        DbType::SQLServer => 4,
        DbType::MongoDB | DbType::Redis => unreachable!("relational databases checked above"),
    };
    if parts.len() > max_parts {
        return Err(SqlBuildError::InvalidIdentifier(format!(
            "{db_type:?} qualified names may have at most {max_parts} components"
        )));
    }
    parts
        .into_iter()
        .map(|part| quote_identifier(db_type, part))
        .collect::<SqlBuildResult<Vec<_>>>()
        .map(|parts| parts.join("."))
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
    ensure_object_support(db_type, kind, "create")?;
    let prefix = match kind {
        ObjectKind::View => "CREATE VIEW",
        ObjectKind::Function => "CREATE FUNCTION",
        ObjectKind::Procedure => "CREATE PROCEDURE",
        ObjectKind::Trigger => "CREATE TRIGGER",
        ObjectKind::Database => "CREATE DATABASE",
    };
    validate_create_sql(sql, prefix)
}

#[cfg(test)]
pub fn build_drop_object(db_type: &DbType, kind: ObjectKind, name: &str) -> SqlBuildResult<String> {
    ensure_object_support(db_type, kind, "drop")?;

    if kind == ObjectKind::Database {
        return Ok(format!(
            "DROP {} {}",
            kind.keyword(),
            quote_identifier(db_type, name)?
        ));
    }

    if kind == ObjectKind::Trigger && matches!(db_type, DbType::PostgreSQL) {
        let mut parts = qualified_parts(name)?;
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

fn sql_string_literal(db_type: &DbType, value: &str) -> SqlBuildResult<String> {
    if value.contains('\0') {
        return Err(SqlBuildError::InvalidInput(
            "SQL string values must not contain NUL".to_string(),
        ));
    }

    Ok(match db_type {
        // Doubling both backslashes and quotes is injection-safe with either
        // MySQL's default escaping or NO_BACKSLASH_ESCAPES mode.
        DbType::MySQL => {
            let escaped = value.replace('\\', "\\\\").replace('\'', "''");
            format!("'{escaped}'")
        }
        // E-strings make backslash handling explicit and independent from the
        // server's standard_conforming_strings setting.
        DbType::PostgreSQL => {
            let escaped = value.replace('\\', "\\\\").replace('\'', "''");
            format!("E'{escaped}'")
        }
        DbType::SQLite => format!("'{}'", value.replace('\'', "''")),
        DbType::SQLServer => format!("N'{}'", value.replace('\'', "''")),
        DbType::ClickHouse => {
            let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
            format!("'{escaped}'")
        }
        DbType::MongoDB | DbType::Redis => {
            return Err(unsupported(db_type, "format structured SQL value"));
        }
    })
}

pub fn sql_literal(db_type: &DbType, value: &Value) -> SqlBuildResult<String> {
    ensure_relational(db_type, "format structured SQL value")?;
    match value {
        Value::Null => Ok("NULL".to_string()),
        Value::Bool(value) => match db_type {
            DbType::PostgreSQL => Ok(if *value { "TRUE" } else { "FALSE" }.to_string()),
            _ => Ok(if *value { "1" } else { "0" }.to_string()),
        },
        Value::Number(value) => Ok(value.to_string()),
        Value::String(value) => sql_string_literal(db_type, value),
        Value::Array(_) | Value::Object(_) => sql_string_literal(db_type, &value.to_string()),
    }
}

pub fn build_insert_row(
    db_type: &DbType,
    table: &str,
    columns: &[String],
    values: &[Value],
) -> SqlBuildResult<String> {
    ensure_relational(db_type, "insert structured row")?;
    if columns.is_empty() {
        return Err(SqlBuildError::InvalidInput(
            "insert requires at least one column".to_string(),
        ));
    }
    if columns.len() != values.len() {
        return Err(SqlBuildError::InvalidInput(format!(
            "insert column/value length mismatch: {} columns, {} values",
            columns.len(),
            values.len()
        )));
    }

    let quoted_columns = columns
        .iter()
        .map(|column| quote_identifier(db_type, column))
        .collect::<SqlBuildResult<Vec<_>>>()?;
    let literals = values
        .iter()
        .map(|value| sql_literal(db_type, value))
        .collect::<SqlBuildResult<Vec<_>>>()?;

    Ok(format!(
        "INSERT INTO {} ({}) VALUES ({})",
        quote_qualified_name(db_type, table)?,
        quoted_columns.join(", "),
        literals.join(", ")
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn build_update_row(
    db_type: &DbType,
    table: &str,
    primary_key_column: &str,
    primary_key_value: &Value,
    column: &str,
    new_value: &Value,
) -> SqlBuildResult<String> {
    ensure_relational(db_type, "update structured row")?;
    if primary_key_value.is_null() {
        return Err(SqlBuildError::InvalidInput(
            "primary-key value must not be null".to_string(),
        ));
    }

    if matches!(db_type, DbType::ClickHouse) {
        Ok(format!(
            "ALTER TABLE {} UPDATE {} = {} WHERE {} = {} SETTINGS mutations_sync = 1",
            quote_qualified_name(db_type, table)?,
            quote_identifier(db_type, column)?,
            sql_literal(db_type, new_value)?,
            quote_identifier(db_type, primary_key_column)?,
            sql_literal(db_type, primary_key_value)?
        ))
    } else {
        Ok(format!(
            "UPDATE {} SET {} = {} WHERE {} = {}",
            quote_qualified_name(db_type, table)?,
            quote_identifier(db_type, column)?,
            sql_literal(db_type, new_value)?,
            quote_identifier(db_type, primary_key_column)?,
            sql_literal(db_type, primary_key_value)?
        ))
    }
}

pub fn build_delete_rows(
    db_type: &DbType,
    table: &str,
    primary_key_column: &str,
    primary_key_values: &[Value],
) -> SqlBuildResult<String> {
    ensure_relational(db_type, "delete structured rows")?;
    if primary_key_values.is_empty() {
        return Err(SqlBuildError::InvalidInput(
            "delete requires at least one primary-key value".to_string(),
        ));
    }
    if primary_key_values.iter().any(Value::is_null) {
        return Err(SqlBuildError::InvalidInput(
            "primary-key values must not contain null".to_string(),
        ));
    }

    let literals = primary_key_values
        .iter()
        .map(|value| sql_literal(db_type, value))
        .collect::<SqlBuildResult<Vec<_>>>()?;
    if matches!(db_type, DbType::ClickHouse) {
        Ok(format!(
            "ALTER TABLE {} DELETE WHERE {} IN ({}) SETTINGS mutations_sync = 1",
            quote_qualified_name(db_type, table)?,
            quote_identifier(db_type, primary_key_column)?,
            literals.join(", ")
        ))
    } else {
        Ok(format!(
            "DELETE FROM {} WHERE {} IN ({})",
            quote_qualified_name(db_type, table)?,
            quote_identifier(db_type, primary_key_column)?,
            literals.join(", ")
        ))
    }
}

/// Validate one `CREATE` statement against an exact keyword prefix.
pub fn validate_create_sql(sql: &str, expected_prefix: &str) -> SqlBuildResult<String> {
    validate_create_sql_prefixes(sql, &[expected_prefix])
}

fn validate_create_sql_prefixes(sql: &str, expected_prefixes: &[&str]) -> SqlBuildResult<String> {
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
    if !expected_prefixes
        .iter()
        .any(|prefix| has_keyword_prefix(trimmed, prefix))
    {
        return Err(SqlBuildError::InvalidCreateSql(format!(
            "expected statement prefix: {}",
            expected_prefixes.join(" or ")
        )));
    }

    let semicolons = top_level_semicolons(trimmed)?;
    match semicolons.as_slice() {
        [] => Ok(trimmed.to_string()),
        [position] if trimmed[position + 1..].trim().is_empty() => {
            Ok(trimmed[..*position].trim_end().to_string())
        }
        _ => Err(SqlBuildError::InvalidCreateSql(
            "only one CREATE statement is allowed".to_string(),
        )),
    }
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

fn top_level_semicolons(sql: &str) -> SqlBuildResult<Vec<usize>> {
    #[derive(Debug)]
    enum State {
        Normal,
        SingleQuote,
        DoubleQuote,
        Backtick,
        Bracket,
        LineComment,
        BlockComment,
        DollarQuote(String),
    }

    let bytes = sql.as_bytes();
    let mut state = State::Normal;
    let mut positions = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        match &state {
            State::Normal => match bytes[index] {
                b'\'' => {
                    state = State::SingleQuote;
                    index += 1;
                }
                b'"' => {
                    state = State::DoubleQuote;
                    index += 1;
                }
                b'`' => {
                    state = State::Backtick;
                    index += 1;
                }
                b'[' => {
                    state = State::Bracket;
                    index += 1;
                }
                b'-' if bytes.get(index + 1) == Some(&b'-') => {
                    state = State::LineComment;
                    index += 2;
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    state = State::BlockComment;
                    index += 2;
                }
                b'$' => {
                    if let Some(delimiter) = dollar_quote_delimiter(&sql[index..]) {
                        index += delimiter.len();
                        state = State::DollarQuote(delimiter);
                    } else {
                        index += 1;
                    }
                }
                b';' => {
                    positions.push(index);
                    index += 1;
                }
                _ => index += 1,
            },
            State::SingleQuote => {
                if bytes[index] == b'\'' {
                    if bytes.get(index + 1) == Some(&b'\'') {
                        index += 2;
                    } else {
                        state = State::Normal;
                        index += 1;
                    }
                } else {
                    index += 1;
                }
            }
            State::DoubleQuote => {
                if bytes[index] == b'"' {
                    if bytes.get(index + 1) == Some(&b'"') {
                        index += 2;
                    } else {
                        state = State::Normal;
                        index += 1;
                    }
                } else {
                    index += 1;
                }
            }
            State::Backtick => {
                if bytes[index] == b'`' {
                    if bytes.get(index + 1) == Some(&b'`') {
                        index += 2;
                    } else {
                        state = State::Normal;
                        index += 1;
                    }
                } else {
                    index += 1;
                }
            }
            State::Bracket => {
                if bytes[index] == b']' {
                    if bytes.get(index + 1) == Some(&b']') {
                        index += 2;
                    } else {
                        state = State::Normal;
                        index += 1;
                    }
                } else {
                    index += 1;
                }
            }
            State::LineComment => {
                if matches!(bytes[index], b'\n' | b'\r') {
                    state = State::Normal;
                }
                index += 1;
            }
            State::BlockComment => {
                if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    state = State::Normal;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            State::DollarQuote(delimiter) => {
                if bytes[index..].starts_with(delimiter.as_bytes()) {
                    index += delimiter.len();
                    state = State::Normal;
                } else {
                    index += 1;
                }
            }
        }
    }

    match state {
        State::Normal | State::LineComment => Ok(positions),
        _ => Err(SqlBuildError::InvalidCreateSql(
            "CREATE statement contains an unterminated quote or comment".to_string(),
        )),
    }
}

fn dollar_quote_delimiter(sql: &str) -> Option<String> {
    let suffix = sql.strip_prefix('$')?;
    let closing = suffix.find('$')?;
    let tag = &suffix[..closing];
    let mut characters = tag.chars();
    let valid_start = characters
        .next()
        .is_none_or(|character| character == '_' || character.is_ascii_alphabetic());
    if valid_start
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        Some(format!("${tag}$"))
    } else {
        None
    }
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
        assert!(quote_identifier(&DbType::PostgreSQL, "public.users").is_err());
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
            "ALTER TABLE `analytics`.`events` UPDATE `message` = 'it\\'s\\\\ready' WHERE `id` = 7 SETTINGS mutations_sync = 1"
        );
        assert_eq!(
            build_delete_rows(
                &DbType::ClickHouse,
                "analytics.events",
                "id",
                &[json!(7), json!(9)],
            )
            .unwrap(),
            "ALTER TABLE `analytics`.`events` DELETE WHERE `id` IN (7, 9) SETTINGS mutations_sync = 1"
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
