//! SQL safety policy for MCP query tools.
//!
//! Parsing is intentionally fail-closed: callers reject syntax errors, while
//! successfully parsed statements whose effects are not understood are
//! classified as [`QueryRisk::Unknown`] and require confirmation.

#[cfg(test)]
use sqlparser::dialect::GenericDialect;
use sqlparser::{
    ast::{
        ConnectByKind, Distinct, Expr, Function, FunctionArg, FunctionArgExpr,
        FunctionArgumentClause, FunctionArguments, GroupByExpr, GroupByWithModifier, Insert,
        JoinConstraint, JoinOperator, LimitClause, NamedWindowExpr, OrderBy, OrderByKind, Query,
        Select, SelectItem, SelectItemQualifiedWildcardKind, SetExpr, Statement, TableFactor,
        TableWithJoins, TopQuantity, WindowFrameBound, WindowSpec,
    },
    dialect::Dialect,
    parser::Parser,
};

/// The side-effect risk associated with one or more SQL statements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryRisk {
    /// A query that only reads data.
    ReadOnly,
    /// An additive operation, such as `INSERT` or `CREATE`.
    Additive,
    /// An `UPDATE` statement.
    Update,
    /// A `DELETE` statement.
    Delete,
    /// A role, user, or privilege change.
    Permissions,
    /// An irreversible or broadly destructive operation.
    Destructive,
    /// A statement that could not be safely classified.
    Unknown,
}

impl QueryRisk {
    /// Returns whether executing this risk class requires user confirmation.
    pub const fn requires_confirmation(self) -> bool {
        !matches!(self, Self::ReadOnly | Self::Additive)
    }

    /// Returns the confirmation category, if confirmation is required.
    pub const fn confirmation_kind(self) -> Option<ConfirmationKind> {
        match self {
            Self::ReadOnly | Self::Additive => None,
            Self::Update => Some(ConfirmationKind::Update),
            Self::Delete => Some(ConfirmationKind::Delete),
            Self::Permissions => Some(ConfirmationKind::Permissions),
            Self::Destructive => Some(ConfirmationKind::Destructive),
            Self::Unknown => Some(ConfirmationKind::Unknown),
        }
    }

    /// Returns a stable, human-readable identifier for logs and tool output.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Additive => "additive",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Permissions => "permissions",
            Self::Destructive => "destructive",
            Self::Unknown => "unknown",
        }
    }

    const fn severity(self) -> u8 {
        match self {
            Self::ReadOnly => 0,
            Self::Additive => 1,
            Self::Update => 2,
            Self::Delete => 3,
            Self::Permissions => 4,
            Self::Destructive => 5,
            // Unknown is most severe so an unclassified statement can never be
            // hidden by another statement in a multi-statement input.
            Self::Unknown => 6,
        }
    }

    const fn highest(self, other: Self) -> Self {
        if self.severity() >= other.severity() {
            self
        } else {
            other
        }
    }
}

/// The kind of confirmation the MCP client must request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationKind {
    Update,
    Delete,
    Permissions,
    Destructive,
    Unknown,
}

impl ConfirmationKind {
    /// Only update confirmations may offer "do not ask again" for this session.
    pub const fn allows_session_suppression(self) -> bool {
        matches!(self, Self::Update)
    }

    /// Returns a stable, human-readable identifier for logs and tool output.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Permissions => "permissions",
            Self::Destructive => "destructive",
            Self::Unknown => "unknown",
        }
    }
}

/// Parsed SQL metadata used by create-query and execute-query guards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlAnalysis {
    pub risk: QueryRisk,
    pub statement_count: usize,
    pub parse_error: Option<String>,
}

impl SqlAnalysis {
    pub const fn requires_confirmation(&self) -> bool {
        self.risk.requires_confirmation()
    }

    pub const fn confirmation_kind(&self) -> Option<ConfirmationKind> {
        self.risk.confirmation_kind()
    }

    /// Returns true only for one successfully parsed SQL statement.
    pub fn is_single_statement(&self) -> bool {
        self.parse_error.is_none() && self.statement_count == 1
    }
}

/// Parses and classifies SQL using sqlparser's generic SQL dialect.
#[cfg(test)]
pub fn analyze_sql(sql: &str) -> SqlAnalysis {
    analyze_sql_with_dialect(sql, &GenericDialect {})
}

/// Parses and classifies SQL using the supplied database dialect.
pub fn analyze_sql_with_dialect(sql: &str, dialect: &dyn Dialect) -> SqlAnalysis {
    match Parser::parse_sql(dialect, sql) {
        Ok(statements) if statements.is_empty() => SqlAnalysis {
            risk: QueryRisk::Unknown,
            statement_count: 0,
            parse_error: Some("SQL contains no statements".to_string()),
        },
        Ok(statements) => SqlAnalysis {
            risk: classify_statements(&statements),
            statement_count: statements.len(),
            parse_error: None,
        },
        Err(error) => SqlAnalysis {
            // sqlparser 0.62 does not represent every vendor-specific
            // identity statement (notably SQL Server CREATE/ALTER LOGIN).
            // Keep the parse error so execution is rejected, while preserving
            // the stricter permissions confirmation category.
            risk: classify_unparsed_security_statement(sql),
            statement_count: 0,
            parse_error: Some(error.to_string()),
        },
    }
}

/// Convenience helper for callers that only need the final risk.
#[cfg(test)]
pub fn classify_sql(sql: &str) -> QueryRisk {
    analyze_sql(sql).risk
}

/// Classifies multiple statements using the highest observed risk.
pub fn classify_statements(statements: &[Statement]) -> QueryRisk {
    statements
        .iter()
        .map(classify_statement)
        .fold(QueryRisk::ReadOnly, QueryRisk::highest)
}

/// Classifies one parsed statement.
pub fn classify_statement(statement: &Statement) -> QueryRisk {
    match statement {
        Statement::Query(query) => classify_query(query),
        Statement::Insert(insert) => classify_insert(insert),
        Statement::Update(_) => QueryRisk::Update,
        Statement::Delete(_) => QueryRisk::Delete,
        Statement::Merge(_) => QueryRisk::Destructive,
        Statement::Grant(_)
        | Statement::Deny(_)
        | Statement::Revoke(_)
        | Statement::CreateRole(_)
        | Statement::AlterRole { .. }
        | Statement::CreateUser(_)
        | Statement::AlterUser(_)
        | Statement::CreatePolicy(_)
        | Statement::AlterPolicy(_) => QueryRisk::Permissions,
        Statement::Truncate(_) | Statement::Drop { .. } => QueryRisk::Destructive,
        Statement::Explain { statement, .. } | Statement::Prepare { statement, .. } => {
            classify_statement(statement)
        }
        Statement::ExplainTable { .. } => QueryRisk::ReadOnly,
        _ => classify_by_normalized_prefix(statement),
    }
}

fn classify_query(query: &Query) -> QueryRisk {
    let mut risk = query
        .with
        .as_ref()
        .map(|with| {
            with.cte_tables
                .iter()
                .map(|cte| classify_query(&cte.query))
                .fold(QueryRisk::ReadOnly, QueryRisk::highest)
        })
        .unwrap_or(QueryRisk::ReadOnly);

    risk = risk.highest(classify_set_expr(&query.body));

    if let Some(order_by) = &query.order_by {
        risk = risk.highest(classify_order_by(order_by));
    }

    if let Some(limit_clause) = &query.limit_clause {
        risk = risk.highest(classify_limit_clause(limit_clause));
    }

    if let Some(fetch) = &query.fetch {
        if let Some(quantity) = &fetch.quantity {
            risk = risk.highest(classify_expr(quantity));
        }
    }

    if let Some(settings) = &query.settings {
        for setting in settings {
            risk = risk.highest(classify_expr(&setting.value));
        }
    }

    // Pipe calls and row locks can have side effects. Fail closed until each
    // variant has an explicit, side-effect-free policy.
    if !query.pipe_operators.is_empty() || !query.locks.is_empty() {
        risk = risk.highest(QueryRisk::Unknown);
    }

    risk
}

fn classify_set_expr(expression: &SetExpr) -> QueryRisk {
    match expression {
        SetExpr::Select(select) => classify_select(select),
        SetExpr::Query(query) => classify_query(query),
        SetExpr::SetOperation { left, right, .. } => {
            classify_set_expr(left).highest(classify_set_expr(right))
        }
        SetExpr::Insert(statement)
        | SetExpr::Update(statement)
        | SetExpr::Delete(statement)
        | SetExpr::Merge(statement) => classify_statement(statement),
        SetExpr::Values(values) => values
            .rows
            .iter()
            .flat_map(|row| row.iter())
            .map(classify_expr)
            .fold(QueryRisk::ReadOnly, QueryRisk::highest),
        SetExpr::Table(_) => QueryRisk::ReadOnly,
    }
}

fn classify_select(select: &Select) -> QueryRisk {
    let mut risk = if select.into.is_some() {
        QueryRisk::Additive
    } else {
        QueryRisk::ReadOnly
    };

    for item in &select.projection {
        risk = risk.highest(classify_select_item(item));
    }

    for table in &select.from {
        risk = risk.highest(classify_table_with_joins(table));
    }

    if let Some(Distinct::On(expressions)) = &select.distinct {
        for expression in expressions {
            risk = risk.highest(classify_expr(expression));
        }
    }

    if let Some(top) = &select.top {
        if let Some(TopQuantity::Expr(quantity)) = &top.quantity {
            risk = risk.highest(classify_expr(quantity));
        }
    }

    for expression in [
        select.prewhere.as_ref(),
        select.selection.as_ref(),
        select.having.as_ref(),
        select.qualify.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        risk = risk.highest(classify_expr(expression));
    }

    risk = risk.highest(classify_group_by(&select.group_by));

    for expression in select
        .cluster_by
        .iter()
        .chain(&select.distribute_by)
        .chain(select.sort_by.iter().map(|order_by| &order_by.expr))
    {
        risk = risk.highest(classify_expr(expression));
    }

    for lateral_view in &select.lateral_views {
        risk = risk.highest(classify_expr(&lateral_view.lateral_view));
    }

    for connect_by in &select.connect_by {
        let connect_risk = match connect_by {
            ConnectByKind::ConnectBy { relationships, .. } => relationships
                .iter()
                .map(classify_expr)
                .fold(QueryRisk::ReadOnly, QueryRisk::highest),
            ConnectByKind::StartWith { condition, .. } => classify_expr(condition),
        };
        risk = risk.highest(connect_risk);
    }

    for definition in &select.named_window {
        if let NamedWindowExpr::WindowSpec(specification) = &definition.1 {
            risk = risk.highest(classify_window_spec(specification));
        }
    }

    risk
}

fn classify_group_by(group_by: &GroupByExpr) -> QueryRisk {
    let (expressions, modifiers) = match group_by {
        GroupByExpr::All(modifiers) => (&[][..], modifiers.as_slice()),
        GroupByExpr::Expressions(expressions, modifiers) => {
            (expressions.as_slice(), modifiers.as_slice())
        }
    };

    let mut risk = expressions
        .iter()
        .map(classify_expr)
        .fold(QueryRisk::ReadOnly, QueryRisk::highest);

    for modifier in modifiers {
        if let GroupByWithModifier::GroupingSets(expression) = modifier {
            risk = risk.highest(classify_expr(expression));
        }
    }

    risk
}

fn classify_order_by(order_by: &OrderBy) -> QueryRisk {
    let mut risk = match &order_by.kind {
        OrderByKind::All(_) => QueryRisk::ReadOnly,
        OrderByKind::Expressions(expressions) => expressions
            .iter()
            .map(classify_order_by_expr)
            .fold(QueryRisk::ReadOnly, QueryRisk::highest),
    };

    if let Some(interpolate) = &order_by.interpolate {
        if let Some(expressions) = &interpolate.exprs {
            for expression in expressions {
                if let Some(value) = &expression.expr {
                    risk = risk.highest(classify_expr(value));
                }
            }
        }
    }

    risk
}

fn classify_order_by_expr(order_by: &sqlparser::ast::OrderByExpr) -> QueryRisk {
    let mut risk = classify_expr(&order_by.expr);

    if let Some(with_fill) = &order_by.with_fill {
        for expression in [
            with_fill.from.as_ref(),
            with_fill.to.as_ref(),
            with_fill.step.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            risk = risk.highest(classify_expr(expression));
        }
    }

    risk
}

fn classify_limit_clause(limit_clause: &LimitClause) -> QueryRisk {
    match limit_clause {
        LimitClause::LimitOffset {
            limit,
            offset,
            limit_by,
        } => {
            let mut risk = limit
                .as_ref()
                .map(classify_expr)
                .unwrap_or(QueryRisk::ReadOnly);
            if let Some(offset) = offset {
                risk = risk.highest(classify_expr(&offset.value));
            }
            for expression in limit_by {
                risk = risk.highest(classify_expr(expression));
            }
            risk
        }
        LimitClause::OffsetCommaLimit { offset, limit } => {
            classify_expr(offset).highest(classify_expr(limit))
        }
    }
}

fn classify_window_spec(specification: &WindowSpec) -> QueryRisk {
    let mut risk = specification
        .partition_by
        .iter()
        .map(classify_expr)
        .fold(QueryRisk::ReadOnly, QueryRisk::highest);

    for order_by in &specification.order_by {
        risk = risk.highest(classify_order_by_expr(order_by));
    }

    if let Some(frame) = &specification.window_frame {
        for bound in [
            &frame.start_bound,
            frame.end_bound.as_ref().unwrap_or(&frame.start_bound),
        ] {
            if let WindowFrameBound::Preceding(Some(expression))
            | WindowFrameBound::Following(Some(expression)) = bound
            {
                risk = risk.highest(classify_expr(expression));
            }
        }
    }

    risk
}

fn classify_select_item(item: &SelectItem) -> QueryRisk {
    match item {
        SelectItem::UnnamedExpr(expression)
        | SelectItem::ExprWithAlias {
            expr: expression, ..
        }
        | SelectItem::ExprWithAliases {
            expr: expression, ..
        } => classify_expr(expression),
        SelectItem::QualifiedWildcard(SelectItemQualifiedWildcardKind::Expr(expression), _) => {
            classify_expr(expression)
        }
        SelectItem::QualifiedWildcard(_, _) | SelectItem::Wildcard(_) => QueryRisk::ReadOnly,
    }
}

fn classify_table_with_joins(table: &TableWithJoins) -> QueryRisk {
    table
        .joins
        .iter()
        .fold(classify_table_factor(&table.relation), |risk, join| {
            risk.highest(classify_table_factor(&join.relation))
                .highest(classify_join_operator(&join.join_operator))
        })
}

fn classify_join_operator(operator: &JoinOperator) -> QueryRisk {
    match operator {
        JoinOperator::Join(constraint)
        | JoinOperator::Inner(constraint)
        | JoinOperator::Left(constraint)
        | JoinOperator::LeftOuter(constraint)
        | JoinOperator::Right(constraint)
        | JoinOperator::RightOuter(constraint)
        | JoinOperator::FullOuter(constraint)
        | JoinOperator::CrossJoin(constraint)
        | JoinOperator::Semi(constraint)
        | JoinOperator::LeftSemi(constraint)
        | JoinOperator::RightSemi(constraint)
        | JoinOperator::Anti(constraint)
        | JoinOperator::LeftAnti(constraint)
        | JoinOperator::RightAnti(constraint)
        | JoinOperator::StraightJoin(constraint) => classify_join_constraint(constraint),
        JoinOperator::AsOf {
            match_condition,
            constraint,
        } => classify_expr(match_condition).highest(classify_join_constraint(constraint)),
        JoinOperator::CrossApply
        | JoinOperator::OuterApply
        | JoinOperator::ArrayJoin
        | JoinOperator::LeftArrayJoin
        | JoinOperator::InnerArrayJoin => QueryRisk::ReadOnly,
    }
}

fn classify_join_constraint(constraint: &JoinConstraint) -> QueryRisk {
    match constraint {
        JoinConstraint::On(expression) => classify_expr(expression),
        JoinConstraint::Using(_) | JoinConstraint::Natural | JoinConstraint::None => {
            QueryRisk::ReadOnly
        }
    }
}

fn classify_table_factor(table: &TableFactor) -> QueryRisk {
    match table {
        TableFactor::Table {
            args: Some(arguments),
            ..
        } => arguments
            .args
            .iter()
            .map(classify_function_arg)
            .fold(QueryRisk::Unknown, QueryRisk::highest),
        TableFactor::Derived { subquery, .. } => classify_query(subquery),
        TableFactor::TableFunction { expr, .. } => QueryRisk::Unknown.highest(classify_expr(expr)),
        TableFactor::Function { args, .. } => args
            .iter()
            .map(classify_function_arg)
            .fold(QueryRisk::Unknown, QueryRisk::highest),
        TableFactor::UNNEST { array_exprs, .. } => array_exprs
            .iter()
            .map(classify_expr)
            .fold(QueryRisk::Unknown, QueryRisk::highest),
        TableFactor::JsonTable { .. }
        | TableFactor::OpenJsonTable { .. }
        | TableFactor::XmlTable { .. }
        | TableFactor::SemanticView { .. } => QueryRisk::Unknown,
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => classify_table_with_joins(table_with_joins),
        TableFactor::Pivot { table, .. }
        | TableFactor::Unpivot { table, .. }
        | TableFactor::MatchRecognize { table, .. } => classify_table_factor(table),
        _ => QueryRisk::ReadOnly,
    }
}

fn classify_expr(expression: &Expr) -> QueryRisk {
    match expression {
        Expr::Subquery(query)
        | Expr::Exists {
            subquery: query, ..
        } => classify_query(query),
        Expr::InSubquery { expr, subquery, .. } => {
            classify_expr(expr).highest(classify_query(subquery))
        }
        Expr::BinaryOp { left, right, .. }
        | Expr::IsDistinctFrom(left, right)
        | Expr::IsNotDistinctFrom(left, right)
        | Expr::AnyOp { left, right, .. }
        | Expr::AllOp { left, right, .. } => classify_expr(left).highest(classify_expr(right)),
        Expr::InList { expr, list, .. } => list
            .iter()
            .map(classify_expr)
            .fold(classify_expr(expr), QueryRisk::highest),
        Expr::InUnnest {
            expr, array_expr, ..
        } => classify_expr(expr).highest(classify_expr(array_expr)),
        Expr::Between {
            expr, low, high, ..
        } => classify_expr(expr)
            .highest(classify_expr(low))
            .highest(classify_expr(high)),
        Expr::Like { expr, pattern, .. }
        | Expr::ILike { expr, pattern, .. }
        | Expr::SimilarTo { expr, pattern, .. }
        | Expr::RLike { expr, pattern, .. } => classify_expr(expr).highest(classify_expr(pattern)),
        Expr::IsFalse(inner)
        | Expr::IsNotFalse(inner)
        | Expr::IsTrue(inner)
        | Expr::IsNotTrue(inner)
        | Expr::IsNull(inner)
        | Expr::IsNotNull(inner)
        | Expr::IsUnknown(inner)
        | Expr::IsNotUnknown(inner)
        | Expr::UnaryOp { expr: inner, .. }
        | Expr::Nested(inner)
        | Expr::Collate { expr: inner, .. }
        | Expr::OuterJoin(inner)
        | Expr::Prior(inner) => classify_expr(inner),
        Expr::Case {
            operand,
            conditions,
            else_result,
            ..
        } => {
            let mut risk = operand
                .as_deref()
                .map(classify_expr)
                .unwrap_or(QueryRisk::ReadOnly);
            for condition in conditions {
                risk = risk
                    .highest(classify_expr(&condition.condition))
                    .highest(classify_expr(&condition.result));
            }
            if let Some(result) = else_result {
                risk = risk.highest(classify_expr(result));
            }
            risk
        }
        Expr::GroupingSets(groups) | Expr::Cube(groups) | Expr::Rollup(groups) => groups
            .iter()
            .flatten()
            .map(classify_expr)
            .fold(QueryRisk::ReadOnly, QueryRisk::highest),
        Expr::Tuple(expressions) => expressions
            .iter()
            .map(classify_expr)
            .fold(QueryRisk::ReadOnly, QueryRisk::highest),
        Expr::Struct { values, .. } => values
            .iter()
            .map(classify_expr)
            .fold(QueryRisk::ReadOnly, QueryRisk::highest),
        Expr::Named { expr, .. } | Expr::Prefixed { value: expr, .. } => classify_expr(expr),
        Expr::Function(function) => classify_function(function),
        Expr::Identifier(_)
        | Expr::CompoundIdentifier(_)
        | Expr::Value(_)
        | Expr::TypedString(_)
        | Expr::Wildcard(_)
        | Expr::QualifiedWildcard(_, _) => QueryRisk::ReadOnly,
        // New or uncommon expression containers may hide a subquery or
        // function call. They are unsafe until recursively modeled here.
        _ => QueryRisk::Unknown,
    }
}

fn classify_function(function: &Function) -> QueryRisk {
    let mut risk = QueryRisk::Unknown;

    for arguments in [&function.parameters, &function.args] {
        risk = risk.highest(classify_function_arguments(arguments));
    }

    if let Some(filter) = &function.filter {
        risk = risk.highest(classify_expr(filter));
    }

    for order_by in &function.within_group {
        risk = risk.highest(classify_expr(&order_by.expr));
    }

    risk
}

fn classify_function_arguments(arguments: &FunctionArguments) -> QueryRisk {
    match arguments {
        FunctionArguments::None => QueryRisk::ReadOnly,
        FunctionArguments::Subquery(query) => classify_query(query),
        FunctionArguments::List(arguments) => {
            let mut risk = arguments
                .args
                .iter()
                .map(classify_function_arg)
                .fold(QueryRisk::ReadOnly, QueryRisk::highest);

            for clause in &arguments.clauses {
                let clause_risk = match clause {
                    FunctionArgumentClause::OrderBy(expressions) => expressions
                        .iter()
                        .map(|expression| classify_expr(&expression.expr))
                        .fold(QueryRisk::ReadOnly, QueryRisk::highest),
                    FunctionArgumentClause::Limit(expression) => classify_expr(expression),
                    _ => QueryRisk::ReadOnly,
                };
                risk = risk.highest(clause_risk);
            }

            risk
        }
    }
}

fn classify_function_arg(argument: &FunctionArg) -> QueryRisk {
    match argument {
        FunctionArg::Named { arg, .. } | FunctionArg::Unnamed(arg) => {
            classify_function_arg_expr(arg)
        }
        FunctionArg::ExprNamed { name, arg, .. } => {
            classify_expr(name).highest(classify_function_arg_expr(arg))
        }
    }
}

fn classify_function_arg_expr(argument: &FunctionArgExpr) -> QueryRisk {
    match argument {
        FunctionArgExpr::Expr(expression) => classify_expr(expression),
        FunctionArgExpr::QualifiedWildcard(_)
        | FunctionArgExpr::Wildcard
        | FunctionArgExpr::WildcardWithOptions(_) => QueryRisk::ReadOnly,
    }
}

fn classify_insert(insert: &Insert) -> QueryRisk {
    let normalized = insert.to_string().to_ascii_uppercase();

    let direct_risk = if normalized.starts_with("REPLACE ")
        || normalized.starts_with("INSERT OR REPLACE ")
        || normalized.starts_with("INSERT OVERWRITE ")
    {
        QueryRisk::Destructive
    } else if (normalized.contains(" ON CONFLICT") && normalized.contains(" DO UPDATE"))
        || normalized.contains(" ON DUPLICATE KEY UPDATE")
    {
        QueryRisk::Update
    } else {
        QueryRisk::Additive
    };

    let source_risk = insert
        .source
        .as_deref()
        .map(classify_query)
        .unwrap_or(QueryRisk::ReadOnly);

    direct_risk.highest(source_risk)
}

/// Covers version-specific AST variants (for example `DropFunction`) without
/// treating arbitrary statements as safe. The text comes from a successfully
/// parsed AST, not directly from untrusted input.
fn classify_by_normalized_prefix(statement: &Statement) -> QueryRisk {
    let normalized = statement.to_string().to_ascii_uppercase();
    let normalized = normalized.trim_start();

    if normalized.starts_with("DROP ") || normalized.starts_with("TRUNCATE ") {
        return QueryRisk::Destructive;
    }

    if is_permissions_statement(normalized) {
        return QueryRisk::Permissions;
    }

    if normalized.starts_with("CREATE OR REPLACE ") || normalized.starts_with("CREATE OR ALTER ") {
        return QueryRisk::Destructive;
    }

    if normalized.starts_with("CREATE ") {
        return QueryRisk::Additive;
    }

    QueryRisk::Unknown
}

fn classify_unparsed_security_statement(sql: &str) -> QueryRisk {
    let normalized = sql.trim_start().to_ascii_uppercase();
    if is_permissions_statement(&normalized) {
        QueryRisk::Permissions
    } else {
        QueryRisk::Unknown
    }
}

fn is_permissions_statement(normalized: &str) -> bool {
    normalized.starts_with("GRANT ")
        || normalized.starts_with("REVOKE ")
        || normalized.starts_with("DENY ")
        || normalized.starts_with("CREATE ROLE ")
        || normalized.starts_with("CREATE USER ")
        || normalized.starts_with("CREATE LOGIN ")
        || normalized.starts_with("CREATE POLICY ")
        || normalized.starts_with("ALTER ROLE ")
        || normalized.starts_with("ALTER USER ")
        || normalized.starts_with("ALTER LOGIN ")
        || normalized.starts_with("ALTER POLICY ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlparser::dialect::{MsSqlDialect, PostgreSqlDialect};

    #[test]
    fn classifies_read_only_queries() {
        for sql in [
            "SELECT 1",
            "WITH values AS (SELECT 1 AS value) SELECT value FROM values",
            "SELECT 1 UNION SELECT 2",
        ] {
            assert_eq!(classify_sql(sql), QueryRisk::ReadOnly, "{sql}");
        }
    }

    #[test]
    fn function_calls_fail_closed_without_a_safe_function_whitelist() {
        for sql in [
            "SELECT count(*) FROM users",
            "SELECT user_defined_function()",
            "SELECT dblink_exec('remote', 'DELETE FROM users')",
            "SELECT 1 WHERE user_defined_function()",
            "SELECT * FROM user_defined_function()",
        ] {
            let analysis = analyze_sql(sql);
            assert_eq!(analysis.parse_error, None, "{sql}");
            assert_eq!(analysis.risk, QueryRisk::Unknown, "{sql}");
            assert!(analysis.requires_confirmation(), "{sql}");
            assert_eq!(
                analysis.confirmation_kind(),
                Some(ConfirmationKind::Unknown),
                "{sql}"
            );
        }
    }

    #[test]
    fn function_calls_in_query_clauses_fail_closed() {
        for sql in [
            "SELECT 1 ORDER BY nuke()",
            "SELECT department FROM users GROUP BY nuke()",
            "SELECT users.id FROM users JOIN accounts ON nuke()",
        ] {
            let analysis = analyze_sql(sql);
            assert_eq!(analysis.parse_error, None, "{sql}");
            assert_eq!(analysis.risk, QueryRisk::Unknown, "{sql}");
            assert_eq!(
                analysis.confirmation_kind(),
                Some(ConfirmationKind::Unknown),
                "{sql}"
            );
        }
    }

    #[test]
    fn classifies_data_modifying_ctes_by_their_nested_statement() {
        let dialect = PostgreSqlDialect {};
        let cases = [
            (
                "WITH added AS (\
                    INSERT INTO users (name) VALUES ('Ada') RETURNING id\
                 ) SELECT id FROM added",
                QueryRisk::Additive,
            ),
            (
                "WITH changed AS (\
                    UPDATE users SET active = false WHERE id = 1 RETURNING id\
                 ) SELECT id FROM changed",
                QueryRisk::Update,
            ),
            (
                "WITH removed AS (\
                    DELETE FROM users WHERE id = 1 RETURNING id\
                 ) SELECT id FROM removed",
                QueryRisk::Delete,
            ),
        ];

        for (sql, expected) in cases {
            let analysis = analyze_sql_with_dialect(sql, &dialect);
            assert_eq!(analysis.parse_error, None, "{sql}");
            assert_eq!(analysis.risk, expected, "{sql}");
        }
    }

    #[test]
    fn classifies_with_wrapped_top_level_update() {
        let dialect = PostgreSqlDialect {};
        let analysis = analyze_sql_with_dialect(
            "WITH selected AS (SELECT 1 AS id) \
             UPDATE users SET active = false FROM selected \
             WHERE users.id = selected.id",
            &dialect,
        );

        assert_eq!(analysis.parse_error, None);
        assert_eq!(analysis.risk, QueryRisk::Update);
    }

    #[test]
    fn classifies_write_in_derived_query() {
        let dialect = PostgreSqlDialect {};
        let analysis = analyze_sql_with_dialect(
            "SELECT id FROM (\
                UPDATE users SET active = false WHERE id = 1 RETURNING id\
             ) AS changed",
            &dialect,
        );

        assert_eq!(analysis.parse_error, None);
        assert_eq!(analysis.risk, QueryRisk::Update);
    }

    #[test]
    fn classifies_write_in_scalar_subquery() {
        let dialect = PostgreSqlDialect {};
        let analysis = analyze_sql_with_dialect(
            "SELECT EXISTS (\
                WITH removed AS (\
                    DELETE FROM users WHERE id = 1 RETURNING id\
                ) SELECT id FROM removed\
             )",
            &dialect,
        );

        assert_eq!(analysis.parse_error, None);
        assert_eq!(analysis.risk, QueryRisk::Delete);
    }

    #[test]
    fn highest_data_modifying_cte_risk_wins() {
        let dialect = PostgreSqlDialect {};
        let analysis = analyze_sql_with_dialect(
            "WITH changed AS (\
                UPDATE users SET active = false WHERE id = 1 RETURNING id\
             ), removed AS (\
                DELETE FROM sessions WHERE user_id IN (SELECT id FROM changed) RETURNING user_id\
             ) SELECT user_id FROM removed",
            &dialect,
        );

        assert_eq!(analysis.parse_error, None);
        assert_eq!(analysis.risk, QueryRisk::Delete);
    }

    #[test]
    fn explain_wrapped_statements_inherit_the_inner_risk() {
        let dialect = PostgreSqlDialect {};
        let cases = [
            ("EXPLAIN SELECT * FROM users", QueryRisk::ReadOnly),
            (
                "EXPLAIN INSERT INTO users (name) VALUES ('Ada')",
                QueryRisk::Additive,
            ),
            (
                "EXPLAIN UPDATE users SET active = false WHERE id = 1",
                QueryRisk::Update,
            ),
            (
                "EXPLAIN ANALYZE DELETE FROM users WHERE id = 1",
                QueryRisk::Delete,
            ),
        ];

        for (sql, expected) in cases {
            let analysis = analyze_sql_with_dialect(sql, &dialect);
            assert_eq!(analysis.parse_error, None, "{sql}");
            assert_eq!(analysis.risk, expected, "{sql}");
        }
    }

    #[test]
    fn prepare_wrapped_update_inherits_the_inner_risk() {
        let dialect = PostgreSqlDialect {};
        let analysis = analyze_sql_with_dialect(
            "PREPARE deactivate AS UPDATE users SET active = false WHERE id = 1",
            &dialect,
        );

        assert_eq!(analysis.parse_error, None);
        assert_eq!(analysis.risk, QueryRisk::Update);
    }

    #[test]
    fn classifies_insert_as_additive() {
        assert_eq!(
            classify_sql("INSERT INTO users (name) VALUES ('Ada')"),
            QueryRisk::Additive
        );
    }

    #[test]
    fn classifies_upsert_as_update() {
        let dialect = PostgreSqlDialect {};
        let analysis = analyze_sql_with_dialect(
            "INSERT INTO users (id, name) VALUES (1, 'Ada') \
             ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name",
            &dialect,
        );

        assert_eq!(analysis.parse_error, None);
        assert_eq!(analysis.risk, QueryRisk::Update);
    }

    #[test]
    fn classifies_common_create_statements_as_additive() {
        for sql in [
            "CREATE TABLE users (id INT)",
            "CREATE SCHEMA analytics",
            "CREATE DATABASE reporting",
        ] {
            assert_eq!(classify_sql(sql), QueryRisk::Additive, "{sql}");
        }
    }

    #[test]
    fn classifies_create_or_replace_as_destructive() {
        let dialect = PostgreSqlDialect {};
        let analysis = analyze_sql_with_dialect(
            "CREATE OR REPLACE VIEW active_users AS SELECT * FROM users",
            &dialect,
        );

        assert_eq!(analysis.parse_error, None);
        assert_eq!(analysis.risk, QueryRisk::Destructive);
    }

    #[test]
    fn classifies_update_and_allows_only_its_confirmation_to_be_suppressed() {
        let analysis = analyze_sql("UPDATE users SET active = false WHERE id = 1");

        assert_eq!(analysis.risk, QueryRisk::Update);
        assert!(analysis.requires_confirmation());
        assert_eq!(analysis.confirmation_kind(), Some(ConfirmationKind::Update));
        assert!(ConfirmationKind::Update.allows_session_suppression());
    }

    #[test]
    fn classifies_delete_as_non_suppressible() {
        let analysis = analyze_sql("DELETE FROM users WHERE id = 1");

        assert_eq!(analysis.risk, QueryRisk::Delete);
        assert_eq!(analysis.confirmation_kind(), Some(ConfirmationKind::Delete));
        assert!(!ConfirmationKind::Delete.allows_session_suppression());
    }

    #[test]
    fn classifies_permission_changes() {
        let dialect = PostgreSqlDialect {};

        for sql in [
            "GRANT SELECT ON users TO analyst",
            "REVOKE SELECT ON users FROM analyst",
            "ALTER ROLE analyst WITH LOGIN",
            "CREATE USER ada",
            "CREATE POLICY active_users ON users USING (active)",
            "ALTER POLICY active_users ON users TO analyst",
        ] {
            let analysis = analyze_sql_with_dialect(sql, &dialect);
            assert_eq!(analysis.parse_error, None, "{sql}");
            assert_eq!(analysis.risk, QueryRisk::Permissions, "{sql}");
        }
    }

    #[test]
    fn classifies_sql_server_login_ddl_as_permissions() {
        let dialect = MsSqlDialect {};

        for sql in [
            "CREATE LOGIN analyst WITH PASSWORD = 'disposable-test-password'",
            "ALTER LOGIN analyst DISABLE",
        ] {
            let analysis = analyze_sql_with_dialect(sql, &dialect);
            assert_eq!(analysis.risk, QueryRisk::Permissions, "{sql}");
            assert_eq!(
                analysis.confirmation_kind(),
                Some(ConfirmationKind::Permissions),
                "{sql}"
            );
        }
    }

    #[test]
    fn classifies_drop_and_truncate_as_destructive() {
        for sql in [
            "DROP TABLE users",
            "DROP FUNCTION refresh_cache",
            "TRUNCATE users",
        ] {
            assert_eq!(classify_sql(sql), QueryRisk::Destructive, "{sql}");
        }
    }

    #[test]
    fn treats_parseable_but_unrecognized_statements_as_unknown() {
        let analysis = analyze_sql("VACUUM");

        assert_eq!(analysis.parse_error, None);
        assert_eq!(analysis.risk, QueryRisk::Unknown);
        assert!(analysis.requires_confirmation());
        assert_eq!(
            analysis.confirmation_kind(),
            Some(ConfirmationKind::Unknown)
        );
    }

    #[test]
    fn parse_errors_fail_closed() {
        let analysis = analyze_sql("SELECT FROM");

        assert_eq!(analysis.risk, QueryRisk::Unknown);
        assert_eq!(analysis.statement_count, 0);
        assert!(analysis.parse_error.is_some());
        assert!(!analysis.is_single_statement());
    }

    #[test]
    fn empty_sql_fails_closed() {
        let analysis = analyze_sql(" \n\t ");

        assert_eq!(analysis.risk, QueryRisk::Unknown);
        assert_eq!(analysis.statement_count, 0);
        assert_eq!(
            analysis.parse_error.as_deref(),
            Some("SQL contains no statements")
        );
    }

    #[test]
    fn reports_single_statement_metadata() {
        let analysis = analyze_sql("SELECT 1");

        assert_eq!(analysis.statement_count, 1);
        assert_eq!(analysis.parse_error, None);
        assert!(analysis.is_single_statement());
    }

    #[test]
    fn multi_statement_sql_uses_the_highest_risk() {
        let analysis = analyze_sql(
            "SELECT 1; INSERT INTO users (name) VALUES ('Ada'); \
             UPDATE users SET active = false WHERE name = 'Ada'",
        );

        assert_eq!(analysis.statement_count, 3);
        assert_eq!(analysis.risk, QueryRisk::Update);
        assert!(!analysis.is_single_statement());
    }

    #[test]
    fn destructive_statement_dominates_other_known_risks() {
        let analysis = analyze_sql("DELETE FROM users; DROP TABLE users");

        assert_eq!(analysis.statement_count, 2);
        assert_eq!(analysis.risk, QueryRisk::Destructive);
    }

    #[test]
    fn unknown_statement_dominates_multi_statement_input() {
        let analysis = analyze_sql("DROP TABLE users; VACUUM");

        assert_eq!(analysis.statement_count, 2);
        assert_eq!(analysis.risk, QueryRisk::Unknown);
    }

    #[test]
    fn safe_risks_do_not_request_confirmation() {
        for risk in [QueryRisk::ReadOnly, QueryRisk::Additive] {
            assert!(!risk.requires_confirmation());
            assert_eq!(risk.confirmation_kind(), None);
        }
    }

    #[test]
    fn identifiers_are_stable() {
        assert_eq!(QueryRisk::ReadOnly.as_str(), "read_only");
        assert_eq!(QueryRisk::Destructive.as_str(), "destructive");
        assert_eq!(ConfirmationKind::Permissions.as_str(), "permissions");
    }
}
