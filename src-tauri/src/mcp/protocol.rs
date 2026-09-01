use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::sql;

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum McpObjectType {
    Database,
    View,
    Function,
    Procedure,
    Trigger,
}

impl McpObjectType {
    pub(super) fn keyword(self) -> &'static str {
        match self {
            Self::Database => "DATABASE",
            Self::View => "VIEW",
            Self::Function => "FUNCTION",
            Self::Procedure => "PROCEDURE",
            Self::Trigger => "TRIGGER",
        }
    }

    pub(super) fn into_sql_kind(self) -> sql::ObjectKind {
        match self {
            Self::Database => sql::ObjectKind::Database,
            Self::View => sql::ObjectKind::View,
            Self::Function => sql::ObjectKind::Function,
            Self::Procedure => sql::ObjectKind::Procedure,
            Self::Trigger => sql::ObjectKind::Trigger,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ConnectionIdArgs {
    pub(super) connection_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct DatabaseObjectArgs {
    pub(super) connection_id: String,
    pub(super) database: String,
    pub(super) object_type: McpObjectType,
    #[schemars(description = "Object name returned as a result label")]
    pub(super) object_name: String,
    #[schemars(description = "Exactly one CREATE statement")]
    pub(super) sql: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct DeleteDatabaseObjectArgs {
    pub(super) connection_id: String,
    pub(super) database: String,
    pub(super) object_type: McpObjectType,
    #[schemars(description = "Exactly one DROP statement")]
    pub(super) sql: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct CreateSchemaArgs {
    pub(super) connection_id: String,
    pub(super) database: String,
    pub(super) schema: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct DeleteSchemaArgs {
    pub(super) connection_id: String,
    pub(super) database: String,
    pub(super) schema: String,
    #[schemars(description = "Use CASCADE where the database supports it")]
    pub(super) cascade: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct CreateTableArgs {
    pub(super) connection_id: String,
    pub(super) database: String,
    #[schemars(description = "Qualified table name shown in the result")]
    pub(super) table: String,
    #[schemars(description = "Exactly one CREATE TABLE statement")]
    pub(super) sql: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct DeleteTableArgs {
    pub(super) connection_id: String,
    pub(super) database: String,
    pub(super) table: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct CreateQueryArgs {
    pub(super) query_id: Option<String>,
    pub(super) name: String,
    pub(super) connection_id: String,
    pub(super) database: String,
    #[schemars(description = "Exactly one SQL statement")]
    pub(super) sql: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct QueryIdArgs {
    pub(super) query_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ExecuteQueryArgs {
    pub(super) query_id: String,
    #[schemars(description = "Maximum rows returned to the MCP client; capped at 500")]
    pub(super) max_rows: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ReadRowsArgs {
    pub(super) connection_id: String,
    pub(super) database: String,
    pub(super) table: String,
    pub(super) page: Option<u32>,
    #[schemars(description = "Rows per page; capped at 500")]
    pub(super) page_size: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct InsertRowArgs {
    pub(super) connection_id: String,
    pub(super) database: String,
    pub(super) table: String,
    #[schemars(length(max = 256))]
    pub(super) columns: Vec<String>,
    #[schemars(length(max = 256))]
    pub(super) values: Vec<Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct UpdateRowArgs {
    pub(super) connection_id: String,
    pub(super) database: String,
    pub(super) table: String,
    pub(super) primary_key_column: String,
    pub(super) primary_key_value: Value,
    pub(super) column: String,
    pub(super) new_value: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct DeleteRowsArgs {
    pub(super) connection_id: String,
    pub(super) database: String,
    pub(super) table: String,
    pub(super) primary_key_column: String,
    #[schemars(length(min = 1, max = 1000))]
    pub(super) primary_key_values: Vec<Value>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(super) struct DestructiveApproval {
    #[schemars(description = "Explicitly approve this one destructive operation")]
    pub(super) confirm: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(super) struct UpdateApproval {
    #[schemars(description = "Explicitly approve this update")]
    pub(super) confirm: bool,
    #[schemars(
        description = "Do not ask again for UPDATE operations on this connection and database during the current MCP session"
    )]
    pub(super) do_not_ask_again: bool,
}

rmcp::elicit_safe!(DestructiveApproval, UpdateApproval);
