use crate::db::DbType;
use sqlparser::{
    dialect::{Dialect, GenericDialect, MsSqlDialect, MySqlDialect, PostgreSqlDialect},
    tokenizer::{Token, Tokenizer, Whitespace},
};

#[derive(Default, Clone)]
pub(super) struct CompletionScope {
    pub(super) qualifier: Option<String>,
    pub(super) relation: bool,
    pub(super) using_columns: bool,
    pub(super) tables: Vec<(String, Option<String>)>,
    pub(super) suppressed: bool,
}

impl CompletionScope {
    pub(super) fn parse(text: &str, db_type: DbType) -> Self {
        let dialect = dialect(db_type);
        let Ok(mut tokens) = Tokenizer::new(dialect, text).tokenize() else {
            return Self {
                suppressed: true,
                ..Self::default()
            };
        };
        if matches!(tokens.last(), Some(Token::Whitespace(Whitespace::SingleLineComment { comment, .. })) if !comment.ends_with('\n'))
        {
            return Self {
                suppressed: true,
                ..Self::default()
            };
        }
        // A word touching the cursor is the replacement prefix, not a completed SQL token.
        if text
            .chars()
            .last()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$')
            && matches!(tokens.last(), Some(Token::Word(_)))
        {
            tokens.pop();
        }
        tokens.retain(|token| !matches!(token, Token::Whitespace(_)));
        let mut scope = Self::default();
        let mut stack = Vec::new();
        let mut in_from = false;
        let mut alias_expected = false;
        let mut index = 0;
        while index < tokens.len() {
            let token = &tokens[index];
            match token {
                Token::SemiColon => {
                    scope = Self::default();
                    in_from = false;
                    alias_expected = false;
                    stack.clear();
                }
                Token::LParen => {
                    stack.push((scope.clone(), in_from, alias_expected));
                    scope.relation = false;
                    in_from = false;
                    alias_expected = false;
                }
                Token::RParen => {
                    if let Some((outer, from, alias)) = stack.pop() {
                        scope = outer;
                        scope.using_columns = false;
                        in_from = from;
                        alias_expected = alias;
                    }
                }
                Token::Comma if in_from => {
                    scope.relation = true;
                    alias_expected = false;
                }
                Token::Word(word) => {
                    let keyword = if word.quote_style.is_none() {
                        word.value.to_ascii_uppercase()
                    } else {
                        String::new()
                    };
                    match keyword.as_str() {
                        "SELECT" => {
                            scope = Self::default();
                            in_from = false;
                            alias_expected = false;
                        }
                        "FROM" | "JOIN" | "UPDATE" | "INTO" => {
                            scope.relation = true;
                            scope.using_columns = false;
                            in_from = true;
                            alias_expected = false;
                        }
                        "USING" => {
                            scope.relation = false;
                            scope.using_columns = true;
                            in_from = false;
                            alias_expected = false;
                        }
                        "WHERE" | "ON" | "HAVING" | "SET" | "VALUES" | "RETURNING" | "GROUP"
                        | "ORDER" | "LIMIT" | "OFFSET" | "PREWHERE" => {
                            scope.relation = false;
                            scope.using_columns = false;
                            in_from = false;
                            alias_expected = false;
                        }
                        "AS" if alias_expected => {}
                        "LEFT" | "RIGHT" | "INNER" | "OUTER" | "FULL" | "CROSS" | "NATURAL" => {
                            alias_expected = false;
                        }
                        _ if scope.relation => {
                            let mut name = word.value.clone();
                            while matches!(tokens.get(index + 1), Some(Token::Period)) {
                                if let Some(Token::Word(part)) = tokens.get(index + 2) {
                                    name.push('.');
                                    name.push_str(&part.value);
                                    index += 2;
                                } else {
                                    scope.qualifier = Some(name);
                                    return scope;
                                }
                            }
                            scope.tables.push((name, None));
                            scope.relation = false;
                            alias_expected = true;
                        }
                        _ if alias_expected => {
                            if let Some((_, alias)) = scope.tables.last_mut() {
                                *alias = Some(word.value.clone());
                            }
                            alias_expected = false;
                        }
                        _ => {}
                    }
                    if matches!(tokens.get(index + 1), Some(Token::Period))
                        && index + 2 == tokens.len()
                    {
                        scope.qualifier = Some(word.value.clone());
                    }
                }
                _ => {}
            }
            index += 1;
        }
        scope
    }
}

fn dialect(db_type: DbType) -> &'static dyn Dialect {
    match db_type {
        DbType::PostgreSQL => &PostgreSqlDialect {},
        DbType::MySQL => &MySqlDialect {},
        DbType::SQLServer => &MsSqlDialect {},
        _ => &GenericDialect {},
    }
}

impl CompletionScope {
    pub(super) fn include_following_tables(&mut self, before: &str, after: &str, db_type: DbType) {
        if self.suppressed || self.relation || after.is_empty() {
            return;
        }
        let current_query = following_query_tokens(before, after, db_type);
        let mut in_from = false;
        let mut relation = false;
        let mut index = 0;
        while index < current_query.len() {
            match &current_query[index] {
                Token::Word(word)
                    if word.quote_style.is_none()
                        && matches!(word.value.to_ascii_uppercase().as_str(), "FROM" | "JOIN") =>
                {
                    in_from = true;
                    relation = true;
                }
                Token::Word(word)
                    if word.quote_style.is_none()
                        && matches!(
                            word.value.to_ascii_uppercase().as_str(),
                            "WHERE"
                                | "ON"
                                | "GROUP"
                                | "ORDER"
                                | "HAVING"
                                | "LIMIT"
                                | "OFFSET"
                                | "PREWHERE"
                        ) =>
                {
                    in_from = false;
                    relation = false;
                }
                Token::Comma if in_from => relation = true,
                Token::LParen => relation = false,
                Token::Word(word) if relation => {
                    let mut name = word.value.clone();
                    while let (Some(Token::Period), Some(Token::Word(part))) =
                        (current_query.get(index + 1), current_query.get(index + 2))
                    {
                        name.push('.');
                        name.push_str(&part.value);
                        index += 2;
                    }
                    relation = false;
                    if matches!(current_query.get(index + 1), Some(Token::LParen)) {
                        index += 1;
                        continue;
                    }
                    if matches!(current_query.get(index + 1), Some(Token::Word(word)) if word.quote_style.is_none() && word.value.eq_ignore_ascii_case("AS"))
                    {
                        index += 1;
                    }
                    let alias = match current_query.get(index + 1) {
                        Some(Token::Word(word))
                            if word.keyword == sqlparser::keywords::Keyword::NoKeyword =>
                        {
                            index += 1;
                            Some(word.value.clone())
                        }
                        _ => None,
                    };
                    if !self
                        .tables
                        .iter()
                        .any(|entry| entry == &(name.clone(), alias.clone()))
                    {
                        self.tables.push((name, alias));
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }
}

fn following_query_tokens(before: &str, after: &str, db_type: DbType) -> Vec<Token> {
    let dialect = dialect(db_type);
    let (Ok(prefix), Ok(suffix)) = (
        Tokenizer::new(dialect, before).tokenize(),
        Tokenizer::new(dialect, after).tokenize(),
    ) else {
        return Vec::new();
    };
    let mut depth = 0usize;
    let mut query_depth = None;
    let mut parents = Vec::new();
    for token in prefix {
        match token {
            Token::LParen => {
                parents.push(query_depth);
                depth += 1;
            }
            Token::RParen => {
                depth = depth.saturating_sub(1);
                query_depth = parents.pop().flatten();
            }
            Token::SemiColon => {
                depth = 0;
                query_depth = None;
                parents.clear();
            }
            Token::Word(word)
                if word.quote_style.is_none() && word.value.eq_ignore_ascii_case("SELECT") =>
            {
                query_depth = Some(depth)
            }
            _ => {}
        }
    }
    let Some(query_depth) = query_depth else {
        return Vec::new();
    };
    let mut current_query = Vec::new();
    for token in suffix {
        match &token {
            Token::SemiColon => break,
            Token::RParen => {
                if depth <= query_depth {
                    break;
                }
                depth -= 1;
                continue;
            }
            Token::LParen => {
                if depth == query_depth {
                    current_query.push(Token::LParen);
                }
                depth += 1;
                continue;
            }
            Token::Word(word)
                if depth == query_depth
                    && word.quote_style.is_none()
                    && matches!(
                        word.value.to_ascii_uppercase().as_str(),
                        "UNION" | "EXCEPT" | "INTERSECT"
                    ) =>
            {
                break
            }
            Token::Whitespace(_) => continue,
            _ => {}
        }
        if depth == query_depth {
            current_query.push(token);
        }
    }

    current_query
}

#[cfg(test)]
mod following_tests {
    use super::*;

    #[test]
    fn following_relations_stay_in_the_current_query_block() {
        for (before, after) in [
            ("SELECT ", " (SELECT total FROM orders) FROM users"),
            ("SELECT ", " FROM users; SELECT total FROM orders"),
            ("SELECT ", " FROM users UNION SELECT total FROM orders"),
            ("SELECT ", " /* FROM orders */ FROM users"),
            ("SELECT * FROM orders WHERE EXISTS (SELECT ", " FROM users)"),
            ("SELECT COUNT(", ") FROM users"),
        ] {
            let mut scope = CompletionScope::parse(before, DbType::PostgreSQL);
            scope.include_following_tables(before, after, DbType::PostgreSQL);
            assert_eq!(
                scope.tables,
                vec![("users".into(), None)],
                "{before}|{after}"
            );
        }
    }

    #[test]
    fn following_relations_preserve_qualified_names_and_aliases() {
        let mut scope = CompletionScope::parse("SELECT u.", DbType::PostgreSQL);
        scope.include_following_tables(
            "SELECT u.",
            " FROM public.users AS u JOIN public.orders o ON u.id = o.user_id",
            DbType::PostgreSQL,
        );
        assert_eq!(
            scope.tables,
            vec![
                ("public.users".into(), Some("u".into())),
                ("public.orders".into(), Some("o".into()))
            ]
        );
    }
}
