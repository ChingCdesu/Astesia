use super::{DbType, SqlRenderError, SqlRenderResult};

mod reader;

pub(crate) struct SqlScript {
    statements: Vec<SqlStatement>,
}

struct SqlStatement {
    sql: String,
    source_start: usize,
}

#[derive(Default)]
struct PendingStatement {
    sql: String,
    source_start: Option<usize>,
}

impl SqlScript {
    pub(crate) fn parse(db_type: DbType, source: &str) -> SqlRenderResult<Self> {
        let mut statements = Vec::new();
        reader::scan(db_type, source.as_bytes(), |statement| {
            statements.push(statement);
            Ok(())
        })?;
        Ok(Self { statements })
    }

    pub(crate) fn for_each_statement(
        db_type: DbType,
        source: impl std::io::BufRead,
        mut emit: impl FnMut(String) -> SqlRenderResult<()>,
    ) -> SqlRenderResult<usize> {
        reader::scan(db_type, source, |statement| emit(statement.sql))
    }

    pub(crate) fn into_statements(self) -> Vec<String> {
        self.statements
            .into_iter()
            .map(|statement| statement.sql)
            .collect()
    }

    pub(crate) fn statement_at(&self, cursor_offset: usize) -> Option<&str> {
        self.statements
            .iter()
            .rev()
            .find(|statement| statement.source_start <= cursor_offset)
            .or_else(|| self.statements.first())
            .map(|statement| statement.sql.as_str())
    }
}

impl PendingStatement {
    fn push_char(&mut self, character: char, source_offset: usize) {
        if self.source_start.is_none() && !character.is_whitespace() {
            self.source_start = Some(source_offset);
        }
        self.sql.push(character);
    }

    fn take(&mut self) -> Option<SqlStatement> {
        let mut sql = std::mem::take(&mut self.sql);
        let start = sql.len() - sql.trim_start().len();
        sql.truncate(sql.trim_end().len());
        let source_start = self.source_start.take();
        if sql.is_empty() {
            return None;
        }
        sql.drain(..start);
        Some(SqlStatement {
            sql,
            source_start: source_start.expect("nonempty SQL source position"),
        })
    }

    fn separator(&mut self) {
        if self
            .sql
            .chars()
            .last()
            .is_some_and(|character| !character.is_whitespace())
        {
            self.sql.push(' ');
        }
    }
}

fn invalid_script(message: &str) -> SqlRenderError {
    SqlRenderError::InvalidScript(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_comments_literals_and_postgres_blocks_without_losing_statements() {
        let script = SqlScript::parse(
            DbType::PostgreSQL,
            "-- backup header\n\
             CREATE TABLE \"events\" (\"message\" text);\n\
             INSERT INTO \"events\" VALUES (E'one;two\\\\three');\n\
             DO $$ BEGIN PERFORM 1; PERFORM 2; END $$; -- done",
        )
        .unwrap()
        .into_statements();

        assert_eq!(script.len(), 3);
        assert!(script[0].starts_with("CREATE TABLE"));
        assert!(script[1].contains("one;two"));
        assert!(script[2].starts_with("DO $$"));
        assert!(script[2].contains("PERFORM 1; PERFORM 2;"));
    }

    #[test]
    fn handles_engine_identifier_escaping_and_rejects_unterminated_input() {
        assert_eq!(
            SqlScript::parse(
                DbType::MySQL,
                "INSERT INTO `odd``table` VALUES ('a\\\';b'); SELECT 1;",
            )
            .unwrap()
            .into_statements()
            .len(),
            2
        );
        assert!(SqlScript::parse(DbType::PostgreSQL, "SELECT 'unterminated").is_err());
        assert!(SqlScript::parse(DbType::PostgreSQL, "SELECT 1 /* unterminated").is_err());
    }

    #[test]
    fn selects_the_statement_at_or_before_the_cursor() {
        let source = "-- header\nSELECT 1;\n\n/* gap */\nSELECT '二';";
        let script = SqlScript::parse(DbType::PostgreSQL, source).unwrap();

        assert_eq!(script.statement_at(0), Some("SELECT 1"));
        assert_eq!(
            script.statement_at(source.find("gap").unwrap()),
            Some("SELECT 1")
        );
        assert_eq!(
            script.statement_at(source.find("二").unwrap()),
            Some("SELECT '二'")
        );
    }
}
