use super::DbType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineProfileSpec {
    File {
        default_path: &'static str,
    },
    Network {
        default_host: &'static str,
        default_port: u16,
        default_username: &'static str,
        default_database: Option<&'static str>,
    },
}

impl EngineProfileSpec {
    pub const fn default_endpoint(self) -> &'static str {
        match self {
            Self::File { default_path } => default_path,
            Self::Network { default_host, .. } => default_host,
        }
    }

    pub const fn default_port(self) -> u16 {
        match self {
            Self::File { .. } => 0,
            Self::Network { default_port, .. } => default_port,
        }
    }

    pub const fn default_username(self) -> &'static str {
        match self {
            Self::File { .. } => "",
            Self::Network {
                default_username, ..
            } => default_username,
        }
    }

    pub const fn default_database(self) -> Option<&'static str> {
        match self {
            Self::File { .. } => None,
            Self::Network {
                default_database, ..
            } => default_database,
        }
    }

    pub const fn is_file(self) -> bool {
        matches!(self, Self::File { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexMode {
    None,
    SqlCatalog,
    MongoCatalog,
    PrimaryKeyOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumMode {
    None,
    Catalog,
    InlineType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowMutationMode {
    None,
    StructuredSql,
    RedisKeyValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableCopyMode {
    None,
    SameEngine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplainMode {
    None,
    Standard,
    SqliteQueryPlan,
    SqlServerShowplanAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerformanceMode {
    Native,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineCapabilities {
    pub sql: bool,
    pub schemas: bool,
    pub schema_management: bool,
    pub database_management: bool,
    pub indexes: IndexMode,
    pub constraints: bool,
    pub enum_values: EnumMode,
    pub views: bool,
    pub functions: bool,
    pub procedures: bool,
    pub triggers: bool,
    pub foreign_keys: bool,
    pub users: bool,
    pub row_mutation: RowMutationMode,
    pub data_browser_read_only: bool,
    pub backup: bool,
    pub restore: bool,
    pub table_copy: TableCopyMode,
    pub explain: ExplainMode,
    pub performance: PerformanceMode,
}

impl EngineCapabilities {
    pub const fn for_engine(engine: DbType) -> Self {
        match engine {
            DbType::MySQL => Self {
                sql: true,
                schemas: false,
                schema_management: true,
                database_management: true,
                indexes: IndexMode::SqlCatalog,
                constraints: true,
                enum_values: EnumMode::None,
                views: true,
                functions: true,
                procedures: true,
                triggers: true,
                foreign_keys: true,
                users: true,
                row_mutation: RowMutationMode::StructuredSql,
                data_browser_read_only: false,
                backup: true,
                restore: true,
                table_copy: TableCopyMode::SameEngine,
                explain: ExplainMode::Standard,
                performance: PerformanceMode::Native,
            },
            DbType::PostgreSQL => Self {
                sql: true,
                schemas: true,
                schema_management: true,
                database_management: true,
                indexes: IndexMode::SqlCatalog,
                constraints: true,
                enum_values: EnumMode::Catalog,
                views: true,
                functions: true,
                procedures: true,
                triggers: true,
                foreign_keys: true,
                users: true,
                row_mutation: RowMutationMode::StructuredSql,
                data_browser_read_only: false,
                backup: true,
                restore: true,
                table_copy: TableCopyMode::SameEngine,
                explain: ExplainMode::Standard,
                performance: PerformanceMode::Native,
            },
            DbType::SQLite => Self {
                sql: true,
                schemas: false,
                schema_management: false,
                database_management: false,
                indexes: IndexMode::SqlCatalog,
                constraints: true,
                enum_values: EnumMode::None,
                views: true,
                functions: false,
                procedures: false,
                triggers: true,
                foreign_keys: true,
                users: false,
                row_mutation: RowMutationMode::StructuredSql,
                data_browser_read_only: false,
                backup: true,
                restore: true,
                table_copy: TableCopyMode::SameEngine,
                explain: ExplainMode::SqliteQueryPlan,
                performance: PerformanceMode::Native,
            },
            DbType::SQLServer => Self {
                sql: true,
                schemas: true,
                schema_management: true,
                database_management: true,
                indexes: IndexMode::SqlCatalog,
                constraints: true,
                enum_values: EnumMode::None,
                views: true,
                functions: true,
                procedures: true,
                triggers: true,
                foreign_keys: true,
                users: true,
                row_mutation: RowMutationMode::StructuredSql,
                data_browser_read_only: false,
                backup: true,
                restore: true,
                table_copy: TableCopyMode::SameEngine,
                explain: ExplainMode::SqlServerShowplanAll,
                performance: PerformanceMode::Native,
            },
            DbType::MongoDB => Self {
                sql: false,
                schemas: false,
                schema_management: false,
                database_management: false,
                indexes: IndexMode::MongoCatalog,
                constraints: false,
                enum_values: EnumMode::None,
                views: false,
                functions: false,
                procedures: false,
                triggers: false,
                foreign_keys: false,
                users: false,
                row_mutation: RowMutationMode::None,
                data_browser_read_only: true,
                backup: false,
                restore: false,
                table_copy: TableCopyMode::None,
                explain: ExplainMode::None,
                performance: PerformanceMode::Native,
            },
            DbType::Redis => Self {
                sql: false,
                schemas: false,
                schema_management: false,
                database_management: false,
                indexes: IndexMode::None,
                constraints: false,
                enum_values: EnumMode::None,
                views: false,
                functions: false,
                procedures: false,
                triggers: false,
                foreign_keys: false,
                users: false,
                row_mutation: RowMutationMode::RedisKeyValue,
                data_browser_read_only: false,
                backup: false,
                restore: false,
                table_copy: TableCopyMode::None,
                explain: ExplainMode::None,
                performance: PerformanceMode::Native,
            },
            DbType::ClickHouse => Self {
                sql: true,
                schemas: false,
                schema_management: false,
                database_management: true,
                indexes: IndexMode::PrimaryKeyOnly,
                constraints: false,
                enum_values: EnumMode::InlineType,
                views: true,
                functions: true,
                procedures: false,
                triggers: false,
                foreign_keys: false,
                users: true,
                row_mutation: RowMutationMode::StructuredSql,
                data_browser_read_only: true,
                backup: true,
                restore: true,
                table_copy: TableCopyMode::SameEngine,
                explain: ExplainMode::Standard,
                performance: PerformanceMode::Native,
            },
        }
    }
}

impl DbType {
    pub const fn supports_query_schema(self) -> bool {
        matches!(self, Self::PostgreSQL)
    }

    pub const fn all() -> [Self; 7] {
        [
            Self::MySQL,
            Self::PostgreSQL,
            Self::SQLite,
            Self::SQLServer,
            Self::MongoDB,
            Self::Redis,
            Self::ClickHouse,
        ]
    }

    pub const fn profile_spec(self) -> EngineProfileSpec {
        match self {
            Self::MySQL => EngineProfileSpec::Network {
                default_port: 3306,
                default_host: "localhost",
                default_username: "root",
                default_database: None,
            },
            Self::PostgreSQL => EngineProfileSpec::Network {
                default_port: 5432,
                default_host: "localhost",
                default_username: "postgres",
                default_database: None,
            },
            Self::SQLite => EngineProfileSpec::File { default_path: "" },
            Self::SQLServer => EngineProfileSpec::Network {
                default_port: 1433,
                default_host: "localhost",
                default_username: "",
                default_database: None,
            },
            Self::MongoDB => EngineProfileSpec::Network {
                default_port: 27017,
                default_host: "localhost",
                default_username: "",
                default_database: None,
            },
            Self::Redis => EngineProfileSpec::Network {
                default_port: 6379,
                default_host: "localhost",
                default_username: "",
                default_database: None,
            },
            Self::ClickHouse => EngineProfileSpec::Network {
                default_port: 8123,
                default_host: "localhost",
                default_username: "default",
                default_database: Some("default"),
            },
        }
    }

    pub const fn capabilities(self) -> EngineCapabilities {
        EngineCapabilities::for_engine(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_distinguish_catalog_shape_from_workflow_policy() {
        let mongo = EngineCapabilities::for_engine(DbType::MongoDB);
        assert_eq!(mongo.indexes, IndexMode::MongoCatalog);
        assert_eq!(mongo.row_mutation, RowMutationMode::None);
        assert!(mongo.data_browser_read_only);
        assert_eq!(mongo.performance, PerformanceMode::Native);

        let clickhouse = EngineCapabilities::for_engine(DbType::ClickHouse);
        assert_eq!(clickhouse.indexes, IndexMode::PrimaryKeyOnly);
        assert_eq!(clickhouse.enum_values, EnumMode::InlineType);
        assert!(!clickhouse.constraints);
        assert_eq!(clickhouse.row_mutation, RowMutationMode::StructuredSql);
        assert!(clickhouse.data_browser_read_only);

        let postgres = EngineCapabilities::for_engine(DbType::PostgreSQL);
        assert!(postgres.sql);
        assert!(postgres.schemas);
        assert!(postgres.schema_management);
        assert!(postgres.database_management);
        assert!(postgres.constraints);
        assert_eq!(postgres.enum_values, EnumMode::Catalog);
        assert_eq!(postgres.table_copy, TableCopyMode::SameEngine);

        let mysql = EngineCapabilities::for_engine(DbType::MySQL);
        assert!(!mysql.schemas);
        assert!(mysql.schema_management);

        let sql_server = EngineCapabilities::for_engine(DbType::SQLServer);
        assert!(sql_server.schemas);
        assert!(sql_server.schema_management);

        let clickhouse = EngineCapabilities::for_engine(DbType::ClickHouse);
        assert!(!clickhouse.schema_management);
        assert!(clickhouse.database_management);

        let redis = EngineCapabilities::for_engine(DbType::Redis);
        assert!(!redis.sql);
        assert_eq!(redis.indexes, IndexMode::None);
        assert_eq!(redis.row_mutation, RowMutationMode::RedisKeyValue);
        assert_eq!(redis.explain, ExplainMode::None);
    }

    #[test]
    fn profile_specs_centralize_engine_defaults() {
        assert_eq!(DbType::all().len(), 7);
        assert_eq!(DbType::PostgreSQL.profile_spec().default_port(), 5432);
        assert!(DbType::SQLite.profile_spec().is_file());
        assert_eq!(
            DbType::ClickHouse.profile_spec().default_database(),
            Some("default")
        );
    }
}
