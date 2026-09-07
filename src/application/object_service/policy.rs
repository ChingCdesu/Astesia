use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ObjectCreationPolicy {
    pub(crate) default_column_type: &'static str,
    pub(crate) default_return_type: &'static str,
    pub(crate) default_routine_language: &'static str,
    pub(crate) function_return_type: bool,
    pub(crate) function_language: bool,
    pub(crate) procedure_language: bool,
    pub(crate) user_host: bool,
    pub(crate) default_trigger_timing: TriggerTiming,
}

pub(crate) const fn object_creation_policy(db_type: DbType) -> ObjectCreationPolicy {
    match db_type {
        DbType::MySQL => ObjectCreationPolicy {
            default_column_type: "INT",
            default_return_type: "INT",
            default_routine_language: "SQL",
            function_return_type: true,
            function_language: false,
            procedure_language: false,
            user_host: true,
            default_trigger_timing: TriggerTiming::Before,
        },
        DbType::PostgreSQL => ObjectCreationPolicy {
            default_column_type: "INTEGER",
            default_return_type: "void",
            default_routine_language: "plpgsql",
            function_return_type: true,
            function_language: true,
            procedure_language: true,
            user_host: false,
            default_trigger_timing: TriggerTiming::Before,
        },
        DbType::SQLite => ObjectCreationPolicy {
            default_column_type: "INTEGER",
            default_return_type: "INT",
            default_routine_language: "SQL",
            function_return_type: true,
            function_language: false,
            procedure_language: false,
            user_host: false,
            default_trigger_timing: TriggerTiming::Before,
        },
        DbType::SQLServer => ObjectCreationPolicy {
            default_column_type: "INT",
            default_return_type: "INT",
            default_routine_language: "T-SQL",
            function_return_type: true,
            function_language: false,
            procedure_language: false,
            user_host: false,
            default_trigger_timing: TriggerTiming::After,
        },
        DbType::ClickHouse => ObjectCreationPolicy {
            default_column_type: "UInt64",
            default_return_type: "",
            default_routine_language: "SQL",
            function_return_type: false,
            function_language: false,
            procedure_language: false,
            user_host: false,
            default_trigger_timing: TriggerTiming::Before,
        },
        DbType::MongoDB | DbType::Redis => ObjectCreationPolicy {
            default_column_type: "TEXT",
            default_return_type: "INT",
            default_routine_language: "SQL",
            function_return_type: true,
            function_language: false,
            procedure_language: false,
            user_host: false,
            default_trigger_timing: TriggerTiming::Before,
        },
    }
}

pub(crate) fn trigger_timing_supported(db_type: DbType, timing: TriggerTiming) -> bool {
    matches!(
        (db_type, timing),
        (
            DbType::PostgreSQL | DbType::MySQL | DbType::SQLite,
            TriggerTiming::Before
        ) | (
            DbType::PostgreSQL | DbType::MySQL | DbType::SQLite | DbType::SQLServer,
            TriggerTiming::After
        ) | (
            DbType::PostgreSQL | DbType::SQLite | DbType::SQLServer,
            TriggerTiming::InsteadOf
        )
    )
}

pub(crate) fn trigger_event_supported(db_type: DbType, event: TriggerEvent) -> bool {
    match event {
        TriggerEvent::Insert | TriggerEvent::Update | TriggerEvent::Delete => {
            db_type.capabilities().triggers
        }
        TriggerEvent::Truncate => db_type == DbType::PostgreSQL,
    }
}

pub(crate) const fn trigger_uses_function_reference(db_type: DbType) -> bool {
    matches!(db_type, DbType::PostgreSQL)
}
