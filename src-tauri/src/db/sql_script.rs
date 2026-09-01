use super::{DbType, SqlRenderError, SqlRenderResult};

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
        if source.contains('\0') {
            return Err(invalid_script("SQL script must not contain NUL"));
        }

        let mut statements = Vec::new();
        let mut statement = PendingStatement::default();
        let mut state = ScanState::Normal;
        let mut index = 0;
        while index < source.len() {
            match &state {
                ScanState::Normal => match source.as_bytes()[index] {
                    b'\'' => {
                        statement.push_char('\'', index);
                        state = ScanState::SingleQuote {
                            backslash_escapes: string_uses_backslash_escapes(
                                db_type, source, index,
                            ),
                        };
                        index += 1;
                    }
                    b'"' => {
                        statement.push_char('"', index);
                        state = ScanState::DoubleQuote;
                        index += 1;
                    }
                    b'`' => {
                        statement.push_char('`', index);
                        state = ScanState::Backtick;
                        index += 1;
                    }
                    b'[' if db_type == DbType::SQLServer => {
                        statement.push_char('[', index);
                        state = ScanState::Bracket;
                        index += 1;
                    }
                    b'-' if source.as_bytes().get(index + 1) == Some(&b'-') => {
                        push_separator(&mut statement);
                        state = ScanState::LineComment;
                        index += 2;
                    }
                    b'#' if db_type == DbType::MySQL => {
                        push_separator(&mut statement);
                        state = ScanState::LineComment;
                        index += 1;
                    }
                    b'/' if source.as_bytes().get(index + 1) == Some(&b'*') => {
                        push_separator(&mut statement);
                        state = ScanState::BlockComment;
                        index += 2;
                    }
                    b'$' if db_type == DbType::PostgreSQL => {
                        if let Some(delimiter) = dollar_quote_delimiter(&source[index..]) {
                            statement.push_str(&delimiter, index);
                            index += delimiter.len();
                            state = ScanState::DollarQuote(delimiter);
                        } else {
                            index = push_source_char(source, index, &mut statement);
                        }
                    }
                    b';' => {
                        push_statement(&mut statements, &mut statement);
                        index += 1;
                    }
                    _ => index = push_source_char(source, index, &mut statement),
                },
                ScanState::SingleQuote { backslash_escapes } => {
                    let byte = source.as_bytes()[index];
                    index = push_source_char(source, index, &mut statement);
                    if *backslash_escapes && byte == b'\\' && index < source.len() {
                        index = push_source_char(source, index, &mut statement);
                    } else if byte == b'\'' {
                        if source.as_bytes().get(index) == Some(&b'\'') {
                            statement.push_char('\'', index);
                            index += 1;
                        } else {
                            state = ScanState::Normal;
                        }
                    }
                }
                ScanState::DoubleQuote => {
                    let byte = source.as_bytes()[index];
                    index = push_source_char(source, index, &mut statement);
                    if byte == b'"' {
                        if source.as_bytes().get(index) == Some(&b'"') {
                            statement.push_char('"', index);
                            index += 1;
                        } else {
                            state = ScanState::Normal;
                        }
                    }
                }
                ScanState::Backtick => {
                    let byte = source.as_bytes()[index];
                    index = push_source_char(source, index, &mut statement);
                    if db_type == DbType::ClickHouse && byte == b'\\' && index < source.len() {
                        index = push_source_char(source, index, &mut statement);
                    } else if byte == b'`' {
                        if source.as_bytes().get(index) == Some(&b'`') {
                            statement.push_char('`', index);
                            index += 1;
                        } else {
                            state = ScanState::Normal;
                        }
                    }
                }
                ScanState::Bracket => {
                    let byte = source.as_bytes()[index];
                    index = push_source_char(source, index, &mut statement);
                    if byte == b']' {
                        if source.as_bytes().get(index) == Some(&b']') {
                            statement.push_char(']', index);
                            index += 1;
                        } else {
                            state = ScanState::Normal;
                        }
                    }
                }
                ScanState::LineComment => {
                    if matches!(source.as_bytes()[index], b'\n' | b'\r') {
                        statement.push_char('\n', index);
                        state = ScanState::Normal;
                    }
                    index += 1;
                }
                ScanState::BlockComment => {
                    if source.as_bytes()[index] == b'*'
                        && source.as_bytes().get(index + 1) == Some(&b'/')
                    {
                        state = ScanState::Normal;
                        index += 2;
                    } else {
                        index += 1;
                    }
                }
                ScanState::DollarQuote(delimiter) => {
                    if source[index..].starts_with(delimiter) {
                        statement.push_str(delimiter, index);
                        index += delimiter.len();
                        state = ScanState::Normal;
                    } else {
                        index = push_source_char(source, index, &mut statement);
                    }
                }
            }
        }

        match state {
            ScanState::Normal | ScanState::LineComment => {}
            ScanState::SingleQuote { .. }
            | ScanState::DoubleQuote
            | ScanState::Backtick
            | ScanState::Bracket
            | ScanState::BlockComment
            | ScanState::DollarQuote(_) => {
                return Err(invalid_script(
                    "SQL script contains an unterminated quote or comment",
                ));
            }
        }
        push_statement(&mut statements, &mut statement);
        Ok(Self { statements })
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

    fn push_str(&mut self, value: &str, source_offset: usize) {
        if self.source_start.is_none() && value.chars().any(|character| !character.is_whitespace())
        {
            self.source_start = Some(source_offset);
        }
        self.sql.push_str(value);
    }
}

enum ScanState {
    Normal,
    SingleQuote { backslash_escapes: bool },
    DoubleQuote,
    Backtick,
    Bracket,
    LineComment,
    BlockComment,
    DollarQuote(String),
}

fn push_source_char(source: &str, index: usize, target: &mut PendingStatement) -> usize {
    let character = source[index..]
        .chars()
        .next()
        .expect("index always points inside a UTF-8 string");
    target.push_char(character, index);
    index + character.len_utf8()
}

fn push_separator(statement: &mut PendingStatement) {
    if statement
        .sql
        .chars()
        .last()
        .is_some_and(|character| !character.is_whitespace())
    {
        statement.sql.push(' ');
    }
}

fn push_statement(statements: &mut Vec<SqlStatement>, statement: &mut PendingStatement) {
    let trimmed = statement.sql.trim();
    if !trimmed.is_empty() {
        statements.push(SqlStatement {
            sql: trimmed.to_string(),
            source_start: statement
                .source_start
                .expect("non-empty SQL has a source position"),
        });
    }
    statement.sql.clear();
    statement.source_start = None;
}

fn string_uses_backslash_escapes(db_type: DbType, source: &str, quote_index: usize) -> bool {
    if matches!(db_type, DbType::MySQL | DbType::ClickHouse) {
        return true;
    }
    if db_type != DbType::PostgreSQL || quote_index == 0 {
        return false;
    }

    let prefix = &source[..quote_index];
    let Some((marker_index, marker)) = prefix.char_indices().next_back() else {
        return false;
    };
    if !matches!(marker, 'e' | 'E') {
        return false;
    }
    prefix[..marker_index]
        .chars()
        .next_back()
        .is_none_or(|character| !character.is_ascii_alphanumeric() && character != '_')
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
