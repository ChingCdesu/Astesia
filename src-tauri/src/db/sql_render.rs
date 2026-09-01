use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::DbType;

pub type SqlRenderResult<T> = Result<T, SqlRenderError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlRenderError {
    Unsupported {
        db_type: DbType,
        operation: &'static str,
    },
    InvalidIdentifier(String),
    InvalidInput(String),
    InvalidCreateSql(String),
    InvalidScript(String),
}

impl fmt::Display for SqlRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { db_type, operation } => {
                write!(formatter, "{operation} is not supported for {db_type:?}")
            }
            Self::InvalidIdentifier(message)
            | Self::InvalidInput(message)
            | Self::InvalidCreateSql(message)
            | Self::InvalidScript(message) => formatter.write_str(message),
        }
    }
}

impl Error for SqlRenderError {}

impl From<SqlRenderError> for String {
    fn from(error: SqlRenderError) -> Self {
        error.to_string()
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SqlDialect {
    db_type: DbType,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TableRef {
    name: String,
    schema: Option<String>,
}

struct QualifiedName {
    parts: Vec<String>,
}

impl QualifiedName {
    fn parse(db_type: DbType, name: &str) -> SqlRenderResult<Self> {
        let parts = name.split('.').map(str::to_string).collect::<Vec<_>>();
        if parts.is_empty() {
            return Err(SqlRenderError::InvalidIdentifier(
                "qualified name must not be empty".to_string(),
            ));
        }
        let max_parts = if db_type == DbType::SQLServer { 4 } else { 2 };
        if parts.len() > max_parts {
            return Err(SqlRenderError::InvalidIdentifier(format!(
                "{db_type:?} qualified names may have at most {max_parts} components"
            )));
        }
        for part in &parts {
            validate_identifier_part(part)?;
        }
        Ok(Self { parts })
    }
}

impl TableRef {
    pub fn parse(db_type: DbType, selector: &str) -> SqlRenderResult<Self> {
        if !matches!(db_type, DbType::PostgreSQL | DbType::SQLServer) {
            validate_identifier_part(selector)?;
            return Ok(Self::unqualified(selector));
        }
        let parts = selector.split('.').collect::<Vec<_>>();
        match parts.as_slice() {
            [name] => {
                validate_identifier_part(name)?;
                Ok(Self::unqualified(*name))
            }
            [schema, name] => {
                validate_identifier_part(schema)?;
                validate_identifier_part(name)?;
                Ok(Self::qualified(*schema, *name))
            }
            _ => Err(SqlRenderError::InvalidIdentifier(format!(
                "{db_type:?} table references may contain at most a schema and table"
            ))),
        }
    }

    pub fn unqualified(name: impl Into<String>) -> Self {
        Self::from_parts(None, name.into())
    }

    pub fn qualified(schema: impl Into<String>, name: impl Into<String>) -> Self {
        Self::from_parts(Some(schema.into()), name.into())
    }

    pub fn from_parts(schema: Option<String>, name: String) -> Self {
        Self { name, schema }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn schema(&self) -> Option<&str> {
        self.schema.as_deref()
    }

    pub fn schema_and_table<'a>(&'a self, default_schema: &'a str) -> (&'a str, &'a str) {
        (self.schema.as_deref().unwrap_or(default_schema), &self.name)
    }
}

impl fmt::Display for TableRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(schema) = &self.schema {
            write!(formatter, "{schema}.{}", self.name)
        } else {
            formatter.write_str(&self.name)
        }
    }
}

impl SqlDialect {
    pub(crate) const fn new(db_type: DbType) -> Self {
        Self { db_type }
    }

    pub(crate) fn quote_identifier(self, identifier: &str) -> SqlRenderResult<String> {
        self.require_sql("quote SQL identifier")?;
        validate_identifier_part(identifier)?;
        Ok(self.render_identifier(identifier, IdentifierStyle::Native))
    }

    pub(crate) fn quote_export_identifier(self, identifier: &str) -> SqlRenderResult<String> {
        self.require_sql("quote SQL export identifier")?;
        validate_identifier_part(identifier)?;
        Ok(self.render_identifier(identifier, IdentifierStyle::Export))
    }

    pub(crate) fn quote_qualified_name(self, name: &str) -> SqlRenderResult<String> {
        self.require_sql("quote SQL qualified name")?;
        QualifiedName::parse(self.db_type, name)?
            .parts
            .iter()
            .map(|part| self.quote_identifier(part))
            .collect::<SqlRenderResult<Vec<_>>>()
            .map(|parts| parts.join("."))
    }

    pub(crate) fn quote_table_ref(self, table: &TableRef) -> SqlRenderResult<String> {
        let name = self.quote_identifier(table.name())?;
        if let Some(schema) = table.schema() {
            return Ok(format!("{}.{}", self.quote_identifier(schema)?, name));
        }
        Ok(name)
    }

    pub(crate) fn quote_export_table_ref(self, table: &TableRef) -> SqlRenderResult<String> {
        let name = self.quote_export_identifier(table.name())?;
        if let Some(schema) = table.schema() {
            return Ok(format!(
                "{}.{}",
                self.quote_export_identifier(schema)?,
                name
            ));
        }
        Ok(name)
    }

    pub(crate) fn quote_table(self, table: &str) -> SqlRenderResult<String> {
        self.quote_qualified_name(table)
    }

    pub(crate) fn literal(self, value: &Value) -> SqlRenderResult<String> {
        self.require_sql("format structured SQL value")?;
        match value {
            Value::Null => Ok("NULL".to_string()),
            Value::Bool(value) if self.db_type == DbType::PostgreSQL => {
                Ok(if *value { "TRUE" } else { "FALSE" }.to_string())
            }
            Value::Bool(value) => Ok(if *value { "1" } else { "0" }.to_string()),
            Value::Number(value) => Ok(value.to_string()),
            Value::String(value) => self.string_literal(value),
            Value::Array(_) | Value::Object(_) => self.string_literal(&value.to_string()),
        }
    }

    pub(crate) fn build_insert_row(
        self,
        table: &TableRef,
        columns: &[String],
        values: &[Value],
    ) -> SqlRenderResult<String> {
        self.require_sql("insert structured row")?;
        self.build_insert_row_for(table, columns, values)
    }

    pub(crate) fn build_insert_row_unqualified(
        self,
        table: &str,
        columns: &[String],
        values: &[Value],
    ) -> SqlRenderResult<String> {
        self.require_sql("insert structured row")?;
        let table = TableRef::unqualified(table);
        self.build_insert_row_for(&table, columns, values)
    }

    fn build_insert_row_for(
        self,
        table: &TableRef,
        columns: &[String],
        values: &[Value],
    ) -> SqlRenderResult<String> {
        if columns.is_empty() {
            return Err(SqlRenderError::InvalidInput(
                "insert requires at least one column".to_string(),
            ));
        }
        if columns.len() != values.len() {
            return Err(SqlRenderError::InvalidInput(format!(
                "insert column/value length mismatch: {} columns, {} values",
                columns.len(),
                values.len()
            )));
        }

        let quoted_columns = columns
            .iter()
            .map(|column| self.quote_identifier(column))
            .collect::<SqlRenderResult<Vec<_>>>()?;
        let literals = values
            .iter()
            .map(|value| self.literal(value))
            .collect::<SqlRenderResult<Vec<_>>>()?;
        Ok(format!(
            "INSERT INTO {} ({}) VALUES ({})",
            self.quote_table_ref(table)?,
            quoted_columns.join(", "),
            literals.join(", ")
        ))
    }

    pub(crate) fn build_update_row(
        self,
        table: &TableRef,
        primary_key_column: &str,
        primary_key_value: &Value,
        column: &str,
        new_value: &Value,
    ) -> SqlRenderResult<String> {
        self.require_sql("update structured row")?;
        if primary_key_value.is_null() {
            return Err(SqlRenderError::InvalidInput(
                "primary-key value must not be null".to_string(),
            ));
        }

        let table = self.quote_table_ref(table)?;
        let column = self.quote_identifier(column)?;
        let new_value = self.literal(new_value)?;
        let primary_key_column = self.quote_identifier(primary_key_column)?;
        let primary_key_value = self.literal(primary_key_value)?;
        if self.db_type == DbType::ClickHouse {
            Ok(format!(
                "ALTER TABLE {table} UPDATE {column} = {new_value} WHERE {primary_key_column} = {primary_key_value} SETTINGS mutations_sync = 1"
            ))
        } else {
            Ok(format!(
                "UPDATE {table} SET {column} = {new_value} WHERE {primary_key_column} = {primary_key_value}"
            ))
        }
    }

    pub(crate) fn build_delete_rows(
        self,
        table: &TableRef,
        primary_key_column: &str,
        primary_key_values: &[Value],
    ) -> SqlRenderResult<String> {
        self.require_sql("delete structured rows")?;
        if primary_key_values.is_empty() {
            return Err(SqlRenderError::InvalidInput(
                "delete requires at least one primary-key value".to_string(),
            ));
        }
        if primary_key_values.iter().any(Value::is_null) {
            return Err(SqlRenderError::InvalidInput(
                "primary-key values must not contain null".to_string(),
            ));
        }

        let table = self.quote_table_ref(table)?;
        let primary_key_column = self.quote_identifier(primary_key_column)?;
        let primary_key_values = primary_key_values
            .iter()
            .map(|value| self.literal(value))
            .collect::<SqlRenderResult<Vec<_>>>()?
            .join(", ");
        if self.db_type == DbType::ClickHouse {
            Ok(format!(
                "ALTER TABLE {table} DELETE WHERE {primary_key_column} IN ({primary_key_values}) SETTINGS mutations_sync = 1"
            ))
        } else {
            Ok(format!(
                "DELETE FROM {table} WHERE {primary_key_column} IN ({primary_key_values})"
            ))
        }
    }

    pub(crate) fn retarget_create_table(
        self,
        create_sql: &str,
        new_table_name: &str,
    ) -> SqlRenderResult<String> {
        self.require_sql("retarget CREATE TABLE")?;
        let replacement = self.quote_export_identifier(new_table_name)?;
        let (name_start, name_end) = create_table_name_range(create_sql)?;
        Ok(format!(
            "{}{}{}",
            &create_sql[..name_start],
            replacement,
            &create_sql[name_end..]
        ))
    }

    fn require_sql(self, operation: &'static str) -> SqlRenderResult<()> {
        if self.db_type.capabilities().sql {
            Ok(())
        } else {
            Err(SqlRenderError::Unsupported {
                db_type: self.db_type,
                operation,
            })
        }
    }

    fn render_identifier(self, identifier: &str, style: IdentifierStyle) -> String {
        match (self.db_type, style) {
            (DbType::MySQL, _) | (DbType::SQLite, IdentifierStyle::Export) => {
                quote_backtick(identifier)
            }
            (DbType::ClickHouse, _) => quote_clickhouse_identifier(identifier),
            (DbType::PostgreSQL, _) | (DbType::SQLite, IdentifierStyle::Native) => {
                quote_double(identifier)
            }
            (DbType::SQLServer, _) => quote_bracket(identifier),
            (DbType::MongoDB | DbType::Redis, _) => {
                unreachable!("SQL support was checked before rendering")
            }
        }
    }

    fn string_literal(self, value: &str) -> SqlRenderResult<String> {
        if value.contains('\0') {
            return Err(SqlRenderError::InvalidInput(
                "SQL string values must not contain NUL".to_string(),
            ));
        }

        Ok(match self.db_type {
            DbType::MySQL => {
                let escaped = value.replace('\\', "\\\\").replace('\'', "''");
                format!("'{escaped}'")
            }
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
                unreachable!("SQL support was checked before rendering")
            }
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IdentifierStyle {
    Native,
    Export,
}

fn validate_identifier_part(identifier: &str) -> SqlRenderResult<()> {
    if identifier.is_empty() {
        return Err(SqlRenderError::InvalidIdentifier(
            "identifier must not be empty".to_string(),
        ));
    }
    if identifier.trim() != identifier {
        return Err(SqlRenderError::InvalidIdentifier(format!(
            "identifier must not have leading or trailing whitespace: {identifier:?}"
        )));
    }
    if identifier.chars().count() > 128 {
        return Err(SqlRenderError::InvalidIdentifier(
            "identifier components must not exceed 128 characters".to_string(),
        ));
    }
    if identifier.chars().any(char::is_control) {
        return Err(SqlRenderError::InvalidIdentifier(
            "identifier must not contain control characters".to_string(),
        ));
    }
    if identifier == "*" {
        return Err(SqlRenderError::InvalidIdentifier(
            "wildcards are not valid object identifiers".to_string(),
        ));
    }
    Ok(())
}

fn create_table_name_range(sql: &str) -> SqlRenderResult<(usize, usize)> {
    let bytes = sql.as_bytes();
    let mut cursor = skip_whitespace(bytes, 0);
    cursor = consume_keyword(sql, cursor, "CREATE")?;
    cursor = skip_whitespace(bytes, cursor);

    if let Some(after_modifier) = try_consume_keyword(sql, cursor, "TEMPORARY")
        .or_else(|| try_consume_keyword(sql, cursor, "TEMP"))
    {
        cursor = skip_whitespace(bytes, after_modifier);
    }

    cursor = consume_keyword(sql, cursor, "TABLE")?;
    cursor = skip_whitespace(bytes, cursor);
    if let Some(after_if) = try_consume_keyword(sql, cursor, "IF") {
        cursor = skip_whitespace(bytes, after_if);
        cursor = consume_keyword(sql, cursor, "NOT")?;
        cursor = skip_whitespace(bytes, cursor);
        cursor = consume_keyword(sql, cursor, "EXISTS")?;
        cursor = skip_whitespace(bytes, cursor);
    }

    let name_start = cursor;
    cursor = consume_identifier_part(sql, cursor)?;
    while bytes.get(cursor) == Some(&b'.') {
        cursor = consume_identifier_part(sql, cursor + 1)?;
    }

    if bytes
        .get(cursor)
        .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'(')
    {
        return Err(invalid_create_sql(
            "CREATE TABLE has an invalid table-name boundary",
        ));
    }
    Ok((name_start, cursor))
}

fn consume_identifier_part(sql: &str, start: usize) -> SqlRenderResult<usize> {
    let bytes = sql.as_bytes();
    let Some(&first) = bytes.get(start) else {
        return Err(invalid_create_sql("CREATE TABLE must name a table"));
    };

    match first {
        b'`' | b'"' => consume_quoted_identifier(bytes, start, first),
        b'[' => consume_bracket_identifier(bytes, start),
        _ => {
            let mut cursor = start;
            while let Some(&byte) = bytes.get(cursor) {
                if byte.is_ascii_whitespace() || matches!(byte, b'.' | b'(') {
                    break;
                }
                if matches!(byte, b';' | b'\'' | b'"' | b'`' | b'[' | b']') {
                    return Err(invalid_create_sql(
                        "CREATE TABLE contains an invalid unquoted table name",
                    ));
                }
                cursor += 1;
            }
            if cursor == start {
                Err(invalid_create_sql("CREATE TABLE must name a table"))
            } else {
                Ok(cursor)
            }
        }
    }
}

fn consume_quoted_identifier(bytes: &[u8], start: usize, delimiter: u8) -> SqlRenderResult<usize> {
    let mut cursor = start + 1;
    while let Some(&byte) = bytes.get(cursor) {
        if byte == b'\\' && delimiter == b'`' {
            cursor += 1;
            if bytes.get(cursor).is_none() {
                return Err(invalid_create_sql(
                    "CREATE TABLE contains an unterminated quoted table name",
                ));
            }
            cursor += 1;
        } else if byte == delimiter {
            if bytes.get(cursor + 1) == Some(&delimiter) {
                cursor += 2;
            } else {
                return Ok(cursor + 1);
            }
        } else {
            cursor += 1;
        }
    }
    Err(invalid_create_sql(
        "CREATE TABLE contains an unterminated quoted table name",
    ))
}

fn consume_bracket_identifier(bytes: &[u8], start: usize) -> SqlRenderResult<usize> {
    let mut cursor = start + 1;
    while let Some(&byte) = bytes.get(cursor) {
        if byte == b']' {
            if bytes.get(cursor + 1) == Some(&b']') {
                cursor += 2;
            } else {
                return Ok(cursor + 1);
            }
        } else {
            cursor += 1;
        }
    }
    Err(invalid_create_sql(
        "CREATE TABLE contains an unterminated bracketed table name",
    ))
}

fn skip_whitespace(bytes: &[u8], mut cursor: usize) -> usize {
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    cursor
}

fn consume_keyword(sql: &str, cursor: usize, keyword: &str) -> SqlRenderResult<usize> {
    try_consume_keyword(sql, cursor, keyword)
        .ok_or_else(|| invalid_create_sql(&format!("expected {keyword} in CREATE TABLE statement")))
}

fn try_consume_keyword(sql: &str, cursor: usize, keyword: &str) -> Option<usize> {
    let end = cursor.checked_add(keyword.len())?;
    let candidate = sql.get(cursor..end)?;
    if !candidate.eq_ignore_ascii_case(keyword) {
        return None;
    }
    if sql
        .as_bytes()
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        return None;
    }
    Some(end)
}

fn invalid_create_sql(message: &str) -> SqlRenderError {
    SqlRenderError::InvalidCreateSql(message.to_string())
}

fn quote_backtick(identifier: &str) -> String {
    format!("`{}`", identifier.replace('`', "``"))
}

fn quote_clickhouse_identifier(identifier: &str) -> String {
    let escaped = identifier.replace('\\', "\\\\").replace('`', "\\`");
    format!("`{escaped}`")
}

fn quote_double(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn quote_bracket(identifier: &str) -> String {
    format!("[{}]", identifier.replace(']', "]]"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_identifiers_and_literals_with_one_strict_policy() {
        let sqlite = SqlDialect::new(DbType::SQLite);
        assert_eq!(sqlite.quote_identifier("items").unwrap(), "\"items\"");
        assert_eq!(sqlite.quote_export_identifier("items").unwrap(), "`items`");
        assert_eq!(
            sqlite.quote_identifier("public.items").unwrap(),
            "\"public.items\""
        );
        assert_eq!(
            sqlite.quote_qualified_name("public.items").unwrap(),
            "\"public\".\"items\""
        );

        let sql_server = SqlDialect::new(DbType::SQLServer);
        assert_eq!(
            sql_server.quote_identifier("odd]name").unwrap(),
            "[odd]]name]"
        );
        assert_eq!(
            sql_server
                .literal(&Value::String("it's".to_string()))
                .unwrap(),
            "N'it''s'"
        );
        assert_eq!(
            sql_server
                .quote_qualified_name("server.database.schema.table")
                .unwrap(),
            "[server].[database].[schema].[table]"
        );

        let postgres = SqlDialect::new(DbType::PostgreSQL);
        assert_eq!(
            postgres.quote_table("odd\"schema.odd\"table").unwrap(),
            "\"odd\"\"schema\".\"odd\"\"table\""
        );
        assert_eq!(postgres.literal(&Value::Bool(true)).unwrap(), "TRUE");

        let clickhouse = SqlDialect::new(DbType::ClickHouse);
        assert_eq!(
            clickhouse.quote_identifier(r"odd\`name").unwrap(),
            r"`odd\\\`name`"
        );
        assert_eq!(
            clickhouse
                .literal(&Value::String("it's\\ready".to_string()))
                .unwrap(),
            "'it\\'s\\\\ready'"
        );
    }

    #[test]
    fn table_selector_parsing_only_qualifies_schema_bearing_engines() {
        let sqlite = TableRef::parse(DbType::SQLite, "events.archive").unwrap();
        assert_eq!(sqlite.schema(), None);
        assert_eq!(sqlite.name(), "events.archive");

        let postgres = TableRef::parse(DbType::PostgreSQL, "audit.events").unwrap();
        assert_eq!(postgres.schema_and_table("public"), ("audit", "events"));
    }

    #[test]
    fn rejects_unsafe_or_unsupported_values() {
        let postgres = SqlDialect::new(DbType::PostgreSQL);
        for identifier in ["", " users", "users\n", "*"] {
            assert!(postgres.quote_identifier(identifier).is_err());
        }
        assert!(postgres
            .literal(&Value::String("unsafe\0value".to_string()))
            .is_err());
        assert!(SqlDialect::new(DbType::MongoDB)
            .quote_identifier("users")
            .is_err());
    }

    #[test]
    fn retargets_only_the_create_table_identifier() {
        let clickhouse = SqlDialect::new(DbType::ClickHouse);
        assert_eq!(
            clickhouse
                .retarget_create_table(
                    "CREATE TABLE IF NOT EXISTS `analytics`.`odd\\`name` (`id` UInt64)",
                    "copy",
                )
                .unwrap(),
            "CREATE TABLE IF NOT EXISTS `copy` (`id` UInt64)"
        );

        let postgres = SqlDialect::new(DbType::PostgreSQL);
        assert_eq!(
            postgres
                .retarget_create_table(
                    "CREATE TABLE \"public\".\"events\" (\"source\" text DEFAULT 'events')",
                    "events_copy",
                )
                .unwrap(),
            "CREATE TABLE \"events_copy\" (\"source\" text DEFAULT 'events')"
        );
        assert!(postgres
            .retarget_create_table("DROP TABLE events", "events_copy")
            .is_err());
        assert!(postgres
            .retarget_create_table("CREATE TABLE \"unterminated (id int)", "events_copy")
            .is_err());
    }

    #[test]
    fn unqualified_insert_and_retarget_share_component_semantics() {
        let sqlite = SqlDialect::new(DbType::SQLite);
        let columns = vec!["value".to_string()];
        assert_eq!(
            sqlite
                .build_insert_row_unqualified("copy.with.dot", &columns, &[Value::from(1)])
                .unwrap(),
            "INSERT INTO \"copy.with.dot\" (\"value\") VALUES (1)"
        );
        assert_eq!(
            sqlite
                .retarget_create_table("CREATE TABLE source (value INTEGER)", "copy.with.dot")
                .unwrap(),
            "CREATE TABLE `copy.with.dot` (value INTEGER)"
        );
    }
}
