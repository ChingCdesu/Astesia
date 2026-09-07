use std::{collections::VecDeque, io::BufRead};

use super::{invalid_script, DbType, PendingStatement, SqlRenderResult, SqlStatement};

enum State {
    Normal,
    Quoted { closing: char, escapes: bool },
    LineComment,
    BlockComment,
    DollarQuote(String),
}

struct Source<R> {
    reader: R,
    ahead: VecDeque<char>,
    offset: usize,
    previous: [Option<char>; 2],
}

impl<R: BufRead> Source<R> {
    fn peek(&mut self, index: usize) -> SqlRenderResult<Option<char>> {
        while self.ahead.len() <= index {
            let mut bytes = [0_u8; 4];
            if self.reader.read(&mut bytes[..1]).map_err(read_error)? == 0 {
                return Ok(None);
            }
            let length = match bytes[0] {
                0 => return Err(invalid_script("SQL script must not contain NUL")),
                1..=127 => 1,
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => return Err(invalid_script("SQL script must be valid UTF-8")),
            };
            self.reader
                .read_exact(&mut bytes[1..length])
                .map_err(read_error)?;
            let character = std::str::from_utf8(&bytes[..length])
                .map_err(|_| invalid_script("SQL script must be valid UTF-8"))?
                .chars()
                .next()
                .expect("one UTF-8 character");
            self.ahead.push_back(character);
        }
        Ok(self.ahead.get(index).copied())
    }

    fn next(&mut self) -> SqlRenderResult<Option<char>> {
        let next = self.peek(0)?;
        if let Some(character) = next {
            self.ahead.pop_front();
            self.offset += character.len_utf8();
            self.previous = [self.previous[1], Some(character)];
        }
        Ok(next)
    }

    fn append(&mut self, statement: &mut PendingStatement) -> SqlRenderResult<()> {
        let offset = self.offset;
        if let Some(character) = self.next()? {
            statement.push_char(character, offset);
        }
        Ok(())
    }

    fn dollar_delimiter(&mut self) -> SqlRenderResult<Option<String>> {
        let mut delimiter = String::from("$");
        let mut index = 1;
        while let Some(character) = self.peek(index)? {
            if character == '$' {
                delimiter.push(character);
                return Ok(Some(delimiter));
            }
            if !(character == '_'
                || character.is_ascii_alphabetic()
                || (index > 1 && character.is_ascii_digit()))
            {
                return Ok(None);
            }
            delimiter.push(character);
            index += 1;
        }
        Ok(None)
    }

    fn matches(&mut self, value: &str) -> SqlRenderResult<bool> {
        for (index, character) in value.chars().enumerate() {
            if self.peek(index)? != Some(character) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn read_error(error: std::io::Error) -> super::SqlRenderError {
    invalid_script(&format!("Unable to read SQL script: {error}"))
}

pub(super) fn scan(
    db_type: DbType,
    reader: impl BufRead,
    mut emit: impl FnMut(SqlStatement) -> SqlRenderResult<()>,
) -> SqlRenderResult<usize> {
    let mut source = Source {
        reader,
        ahead: VecDeque::new(),
        offset: 0,
        previous: [None; 2],
    };
    let mut pending = PendingStatement::default();
    let mut state = State::Normal;
    let mut count = 0;
    while let Some(character) = source.peek(0)? {
        match &state {
            State::Normal => match character {
                '\'' | '"' | '`' | '[' if character != '[' || db_type == DbType::SQLServer => {
                    let escapes = (character == '\''
                        && (matches!(db_type, DbType::MySQL | DbType::ClickHouse)
                            || (db_type == DbType::PostgreSQL
                                && matches!(source.previous[1], Some('e' | 'E'))
                                && source.previous[0]
                                    .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_'))))
                        || (character == '`' && db_type == DbType::ClickHouse);
                    state = State::Quoted {
                        closing: if character == '[' { ']' } else { character },
                        escapes,
                    };
                    source.append(&mut pending)?;
                }
                '-' if source.peek(1)? == Some('-') => {
                    pending.separator();
                    source.next()?;
                    source.next()?;
                    state = State::LineComment;
                }
                '#' if db_type == DbType::MySQL => {
                    pending.separator();
                    source.next()?;
                    state = State::LineComment;
                }
                '/' if source.peek(1)? == Some('*') => {
                    pending.separator();
                    source.next()?;
                    source.next()?;
                    state = State::BlockComment;
                }
                '$' if db_type == DbType::PostgreSQL => {
                    if let Some(delimiter) = source.dollar_delimiter()? {
                        for _ in delimiter.chars() {
                            source.append(&mut pending)?;
                        }
                        state = State::DollarQuote(delimiter);
                    } else {
                        source.append(&mut pending)?;
                    }
                }
                ';' => {
                    source.next()?;
                    if let Some(statement) = pending.take() {
                        emit(statement)?;
                        count += 1;
                    }
                }
                _ => source.append(&mut pending)?,
            },
            State::Quoted { closing, escapes } => {
                let closing = *closing;
                let escapes = *escapes;
                source.append(&mut pending)?;
                if escapes && character == '\\' {
                    source.append(&mut pending)?;
                } else if character == closing {
                    if source.peek(0)? == Some(closing) {
                        source.append(&mut pending)?;
                    } else {
                        state = State::Normal;
                    }
                }
            }
            State::LineComment => {
                if matches!(character, '\n' | '\r') {
                    pending.push_char('\n', source.offset);
                    state = State::Normal;
                }
                source.next()?;
            }
            State::BlockComment => {
                if character == '*' && source.peek(1)? == Some('/') {
                    source.next()?;
                    source.next()?;
                    state = State::Normal;
                } else {
                    source.next()?;
                }
            }
            State::DollarQuote(delimiter) => {
                if character == '$' && source.matches(delimiter)? {
                    for _ in delimiter.chars() {
                        source.append(&mut pending)?;
                    }
                    state = State::Normal;
                } else {
                    source.append(&mut pending)?;
                }
            }
        }
    }
    if !matches!(state, State::Normal | State::LineComment) {
        return Err(invalid_script(
            "SQL script contains an unterminated quote or comment",
        ));
    }
    if let Some(statement) = pending.take() {
        emit(statement)?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqlScript;
    use std::io::BufReader;

    #[test]
    fn tokens_cross_every_reader_boundary() {
        let sql = "-- header\nSELECT E'你;好\\\'x', \"a\"\"b\"; DO $body$ BEGIN PERFORM 1; END $body$; /*尾*/ SELECT 2";
        let expected = SqlScript::parse(DbType::PostgreSQL, sql)
            .unwrap()
            .into_statements();
        for capacity in 1..=32 {
            let mut actual = Vec::new();
            SqlScript::for_each_statement(
                DbType::PostgreSQL,
                BufReader::with_capacity(capacity, sql.as_bytes()),
                |statement| {
                    actual.push(statement);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn rejects_invalid_bytes_in_comments_and_incomplete_tail() {
        for bytes in [
            b"SELECT 1; --\xff".as_slice(),
            b"SELECT 1; /*\0*/",
            b"SELECT 1; SELECT 'tail",
        ] {
            assert!(scan(DbType::SQLite, bytes, |_| Ok(())).is_err());
        }
    }
}
