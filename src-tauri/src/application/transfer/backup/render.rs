use chrono::Utc;

use crate::db::{DbType, QueryResult, SqlDialect, SqlRenderResult};

use super::plan::BackupTable;
use super::DropTableMode;

pub(super) struct BackupRenderer {
    db_type: DbType,
    dialect: SqlDialect,
    output: String,
}

impl BackupRenderer {
    pub(super) fn new(db_type: DbType, database: &str) -> Self {
        Self::with_timestamp(
            db_type,
            database,
            &Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        )
    }

    fn with_timestamp(db_type: DbType, database: &str, timestamp: &str) -> Self {
        let mut output = String::new();
        push_sql_comment(&mut output, "Astesia Database Backup");
        push_sql_comment(&mut output, &format!("Database: {database}"));
        push_sql_comment(&mut output, &format!("Date: {timestamp}"));
        output.push('\n');
        match db_type {
            DbType::MySQL => output.push_str("SET FOREIGN_KEY_CHECKS = 0;\n\n"),
            DbType::SQLite => output.push_str("PRAGMA foreign_keys = OFF;\n\n"),
            _ => {}
        }
        Self {
            db_type,
            dialect: SqlDialect::new(db_type),
            output,
        }
    }

    pub(super) fn render_drop_tables(
        &mut self,
        tables: &[BackupTable],
        mode: DropTableMode,
    ) -> SqlRenderResult<()> {
        if mode == DropTableMode::None {
            return Ok(());
        }

        let mut section = String::new();
        for table in tables.iter().rev() {
            if self.db_type == DbType::PostgreSQL {
                for index in table.indexes.iter().filter(|index| !index.is_primary) {
                    let schema =
                        self.quote_identifier(table.reference.schema().unwrap_or("public"))?;
                    let index_name = self.quote_identifier(&index.name)?;
                    if mode == DropTableMode::DropIfExists {
                        section.push_str(&format!("DROP INDEX IF EXISTS {schema}.{index_name};\n"));
                    } else {
                        section.push_str(&format!("DROP INDEX {schema}.{index_name};\n"));
                    }
                }
            }

            let quoted_table = self.quote_table(table)?;
            if mode == DropTableMode::DropIfExists {
                section.push_str(&format!("DROP TABLE IF EXISTS {quoted_table};\n"));
            } else {
                section.push_str(&format!("DROP TABLE {quoted_table};\n"));
            }
        }
        section.push('\n');
        self.output.push_str(&section);
        Ok(())
    }

    pub(super) fn render_structure(
        &mut self,
        table: &BackupTable,
        create_sql: &str,
    ) -> SqlRenderResult<()> {
        let index_statements = if self.db_type == DbType::PostgreSQL {
            self.pg_create_index_statements(table)?
        } else {
            Vec::new()
        };

        self.output.push_str(create_sql);
        self.output.push_str(";\n\n");
        for statement in index_statements {
            self.output.push_str(&statement);
            self.output.push('\n');
        }
        if table.indexes.iter().any(|index| !index.is_primary) && self.db_type == DbType::PostgreSQL
        {
            self.output.push('\n');
        }
        Ok(())
    }

    pub(super) fn render_structure_error(&mut self, table: impl std::fmt::Display, error: &str) {
        push_sql_comment(
            &mut self.output,
            &format!("Error getting DDL for {table}: {error}"),
        );
        self.output.push('\n');
    }

    pub(super) fn render_data_page(
        &mut self,
        table: &BackupTable,
        result: &QueryResult,
    ) -> SqlRenderResult<()> {
        let quoted_table = self.quote_table(table)?;
        let columns = result
            .columns
            .iter()
            .map(|column| self.quote_export_identifier(&column.name))
            .collect::<SqlRenderResult<Vec<_>>>()?;
        let mut page = String::new();
        for row in &result.rows {
            let values = row
                .iter()
                .map(|value| self.dialect.literal(value))
                .collect::<SqlRenderResult<Vec<_>>>()?;
            page.push_str(&format!(
                "INSERT INTO {quoted_table} ({}) VALUES ({});\n",
                columns.join(", "),
                values.join(", ")
            ));
        }
        self.output.push_str(&page);
        Ok(())
    }

    pub(super) fn finish_table_data(&mut self, table: &BackupTable) -> SqlRenderResult<()> {
        let reset_statements = self.reset_auto_increment_statements(table)?;
        self.output.push('\n');
        for statement in reset_statements {
            self.output.push_str(&statement);
            self.output.push('\n');
        }
        self.output.push('\n');
        Ok(())
    }

    pub(super) fn finish_success(&mut self) {
        match self.db_type {
            DbType::MySQL => self.output.push_str("\nSET FOREIGN_KEY_CHECKS = 1;\n"),
            DbType::SQLite => self.output.push_str("\nPRAGMA foreign_keys = ON;\n"),
            _ => {}
        }
    }

    pub(super) fn into_output(self) -> String {
        self.output
    }

    fn quote_identifier(&self, identifier: &str) -> SqlRenderResult<String> {
        self.dialect.quote_identifier(identifier)
    }

    fn quote_export_identifier(&self, identifier: &str) -> SqlRenderResult<String> {
        self.dialect.quote_export_identifier(identifier)
    }

    fn quote_table(&self, table: &BackupTable) -> SqlRenderResult<String> {
        match self.db_type {
            DbType::PostgreSQL => self.quote_postgres_table(table),
            DbType::SQLServer => self.dialect.quote_export_table_ref(&table.reference),
            _ => self.quote_export_identifier(table.reference.name()),
        }
    }

    fn quote_postgres_table(&self, table: &BackupTable) -> SqlRenderResult<String> {
        Ok(format!(
            "{}.{}",
            self.quote_identifier(table.reference.schema().unwrap_or("public"))?,
            self.quote_identifier(table.reference.name())?
        ))
    }

    fn pg_create_index_statements(&self, table: &BackupTable) -> SqlRenderResult<Vec<String>> {
        let quoted_table = self.quote_postgres_table(table)?;
        table
            .indexes
            .iter()
            .filter(|index| !index.is_primary)
            .map(|index| {
                let unique = if index.is_unique { "UNIQUE " } else { "" };
                let columns = index
                    .columns
                    .iter()
                    .map(|column| self.quote_identifier(column))
                    .collect::<SqlRenderResult<Vec<_>>>()?;
                Ok(format!(
                    "CREATE {unique}INDEX {} ON {quoted_table} ({});",
                    self.quote_identifier(&index.name)?,
                    columns.join(", ")
                ))
            })
            .collect()
    }

    fn reset_auto_increment_statements(&self, table: &BackupTable) -> SqlRenderResult<Vec<String>> {
        match self.db_type {
            DbType::PostgreSQL => {
                let qualified_table = self.quote_postgres_table(table)?;
                let select_template = format!("SELECT COALESCE(MAX(%I), 0) FROM {qualified_table}")
                    .replace('\'', "''");
                let regclass = self
                    .dialect
                    .literal(&serde_json::Value::String(qualified_table))?;
                let schema = self.dialect.literal(&serde_json::Value::String(
                    table.reference.schema().unwrap_or("public").to_string(),
                ))?;
                let table_name = self.dialect.literal(&serde_json::Value::String(
                    table.reference.name().to_string(),
                ))?;
                Ok(vec![format!(
                    "DO $$ DECLARE seq RECORD; max_val BIGINT; BEGIN \
                     FOR seq IN SELECT column_name, pg_get_serial_sequence({regclass}, column_name) AS seqname \
                     FROM information_schema.columns WHERE table_schema = {schema} AND table_name = {table_name} \
                     AND pg_get_serial_sequence({regclass}, column_name) IS NOT NULL \
                     LOOP \
                     EXECUTE format('{select_template}', seq.column_name) INTO max_val; \
                     PERFORM setval(seq.seqname, GREATEST(max_val, 1)); \
                     END LOOP; \
                     END $$;"
                )])
            }
            _ => Ok(Vec::new()),
        }
    }
}

fn push_sql_comment(output: &mut String, text: &str) {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    for line in normalized.split('\n') {
        output.push_str("-- ");
        output.push_str(line);
        output.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{ColumnInfo, DbType, IndexInfo, QueryResult, TableRef};

    use super::{BackupRenderer, BackupTable};

    fn table(schema: Option<&str>, name: &str) -> BackupTable {
        BackupTable {
            reference: TableRef::from_parts(schema.map(str::to_string), name.to_string()),
            indexes: Vec::new(),
        }
    }

    #[test]
    fn renders_postgres_indexes_without_primary_keys() {
        let mut table = table(Some("auth"), "users");
        table.indexes = vec![
            IndexInfo {
                name: "users_pkey".to_string(),
                columns: vec!["id".to_string()],
                is_unique: true,
                is_primary: true,
            },
            IndexInfo {
                name: "users_email_key".to_string(),
                columns: vec!["email".to_string()],
                is_unique: true,
                is_primary: false,
            },
        ];
        let renderer = BackupRenderer::with_timestamp(DbType::PostgreSQL, "app", "date");

        assert_eq!(
            renderer.pg_create_index_statements(&table).unwrap(),
            ["CREATE UNIQUE INDEX \"users_email_key\" ON \"auth\".\"users\" (\"email\");"]
        );
    }

    #[test]
    fn only_supported_engines_render_sequence_resets() {
        let table = table(Some("auth"), "users");
        let sqlite = BackupRenderer::with_timestamp(DbType::SQLite, "app", "date");
        let postgres = BackupRenderer::with_timestamp(DbType::PostgreSQL, "app", "date");

        assert!(sqlite
            .reset_auto_increment_statements(&table)
            .unwrap()
            .is_empty());
        assert_eq!(
            postgres
                .reset_auto_increment_statements(&table)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn renders_data_rows_with_export_quoting_and_literals() {
        let table = table(None, "users");
        let result = QueryResult {
            columns: vec![ColumnInfo {
                name: "display name".to_string(),
                data_type: "TEXT".to_string(),
                nullable: false,
                is_primary_key: false,
                default_value: None,
                comment: None,
            }],
            rows: vec![vec![serde_json::Value::String("O'Brien".to_string())]],
            affected_rows: 0,
            execution_time_ms: 0,
        };
        let mut renderer = BackupRenderer::with_timestamp(DbType::SQLite, "app", "date");

        renderer.render_data_page(&table, &result).unwrap();

        assert!(renderer
            .into_output()
            .contains("INSERT INTO `users` (`display name`) VALUES ('O''Brien');"));
    }

    #[test]
    fn keeps_multiline_metadata_and_errors_inside_sql_comments() {
        let mut renderer =
            BackupRenderer::with_timestamp(DbType::SQLite, "app\nDROP DATABASE app;", "date");
        renderer.render_structure_error("users\nDROP TABLE users;", "failed\nDELETE FROM users;");
        let output = renderer.into_output();

        assert!(output.contains("-- Database: app\n-- DROP DATABASE app;\n"));
        assert!(output.contains(
            "-- Error getting DDL for users\n-- DROP TABLE users;: failed\n-- DELETE FROM users;\n"
        ));
    }

    #[test]
    fn renders_dotted_schema_and_table_as_separate_identifiers() {
        let table = table(Some("billing.v2"), "account.history");
        let renderer = BackupRenderer::with_timestamp(DbType::PostgreSQL, "app", "date");

        assert_eq!(
            renderer.quote_table(&table).unwrap(),
            "\"billing.v2\".\"account.history\""
        );
    }
}
