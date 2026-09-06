use super::{GridSort, GridSortDirection};
use sqlparser::{ast::Expr, dialect::GenericDialect, parser::Parser, tokenizer::Token};

impl GridSort {
    pub(crate) fn parse_list(input: &str) -> Result<Vec<Self>, String> {
        if input.trim().is_empty() {
            return Ok(Vec::new());
        }
        let dialect = GenericDialect {};
        let mut parser = Parser::new(&dialect)
            .try_with_sql(input)
            .map_err(|error| error.to_string())?;
        let expressions = parser
            .parse_comma_separated(|parser| parser.parse_order_by_expr())
            .map_err(|error| error.to_string())?;
        parser
            .expect_token(&Token::EOF)
            .map_err(|error| error.to_string())?;
        expressions
            .into_iter()
            .map(|expression| {
                let Expr::Identifier(column) = expression.expr else {
                    return Err("Use column names followed by ASC or DESC".to_string());
                };
                if expression.options.nulls_first.is_some() || expression.with_fill.is_some() {
                    return Err("Only ASC and DESC ordering are supported".to_string());
                }
                Ok(Self {
                    column: column.value,
                    direction: if expression.options.asc == Some(false) {
                        GridSortDirection::Descending
                    } else {
                        GridSortDirection::Ascending
                    },
                })
            })
            .collect()
    }

    pub(crate) fn format_list(sort: &[Self]) -> String {
        sort.iter()
            .map(|sort| {
                format!(
                    "\"{}\" {}",
                    sort.column.replace('"', "\"\""),
                    match sort.direction {
                        GridSortDirection::Ascending => "ASC",
                        GridSortDirection::Descending => "DESC",
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_columns_and_multiple_directions_round_trip() {
        let sort = GridSort::parse_list("\"created at\" DESC, \"a\"\"b\", id ASC").unwrap();
        assert_eq!(sort[0].column, "created at");
        assert_eq!(sort[0].direction, GridSortDirection::Descending);
        assert_eq!(sort[1].column, "a\"b");
        assert_eq!(
            GridSort::parse_list(&GridSort::format_list(&sort)).unwrap(),
            sort
        );
        assert!(GridSort::parse_list("  ").unwrap().is_empty());
    }

    #[test]
    fn extra_statements_and_unsupported_ordering_are_rejected() {
        for input in [
            "id; DELETE FROM users",
            "id LIMIT 10",
            "lower(name)",
            "id NULLS FIRST",
            "1",
        ] {
            assert!(GridSort::parse_list(input).is_err(), "{input}");
        }
    }
}
