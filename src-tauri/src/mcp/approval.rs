use std::time::Duration;

use rmcp::{service::ElicitationError, Peer, RoleServer, ServiceError};
use serde_json::{json, Value};

use super::{
    catalog::{CatalogError, SavedQuery},
    failure::{
        McpToolFailure, ERROR_CODE_APPROVAL_DECLINED, ERROR_CODE_APPROVAL_INVALID_RESPONSE,
        ERROR_CODE_APPROVAL_TIMEOUT, ERROR_CODE_APPROVAL_UNAVAILABLE,
        ERROR_CODE_APPROVAL_UNSUPPORTED, ERROR_CODE_CONNECTION_NOT_FOUND,
    },
    policy::{QueryRisk, SqlAnalysis},
    protocol::{DestructiveApproval, UpdateApproval},
    AstesiaMcp,
};

const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ApprovalKind {
    Destructive,
    Update,
}

impl ApprovalKind {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Destructive => "destructive",
            Self::Update => "update",
        }
    }

    fn blocked_message(self, error: &ElicitationError) -> String {
        match self {
            Self::Destructive => {
                format!("高危操作已阻止：MCP 客户端未提供可用的用户确认（{error}）")
            }
            Self::Update => {
                format!("更新操作已阻止：MCP 客户端未提供可用的用户确认（{error}）")
            }
        }
    }

    const fn missing_content_message(self) -> &'static str {
        match self {
            Self::Destructive => "高危操作已取消：未收到确认内容",
            Self::Update => "更新操作已取消：未收到确认内容",
        }
    }

    const fn declined_message(self) -> &'static str {
        match self {
            Self::Destructive => "用户拒绝确认，高危操作未执行",
            Self::Update => "用户拒绝确认，更新操作未执行",
        }
    }

    const fn cancelled_message(self) -> &'static str {
        match self {
            Self::Destructive => "用户取消确认，高危操作未执行",
            Self::Update => "用户取消确认，更新操作未执行",
        }
    }
}

pub(super) fn approval_details(kind: ApprovalKind) -> Value {
    json!({ "approval_kind": kind.as_str() })
}

pub(super) fn map_elicitation_failure(
    kind: ApprovalKind,
    error: ElicitationError,
) -> McpToolFailure {
    let (code, retryable) = match &error {
        ElicitationError::CapabilityNotSupported => (ERROR_CODE_APPROVAL_UNSUPPORTED, false),
        ElicitationError::UserDeclined => (ERROR_CODE_APPROVAL_DECLINED, false),
        ElicitationError::UserCancelled => (super::failure::ERROR_CODE_APPROVAL_CANCELLED, false),
        ElicitationError::Service(ServiceError::Timeout { .. }) => {
            (ERROR_CODE_APPROVAL_TIMEOUT, true)
        }
        ElicitationError::ParseError { .. } | ElicitationError::NoContent => {
            (ERROR_CODE_APPROVAL_INVALID_RESPONSE, false)
        }
        ElicitationError::Service(_) => (ERROR_CODE_APPROVAL_UNAVAILABLE, true),
        _ => (ERROR_CODE_APPROVAL_UNAVAILABLE, true),
    };
    let message = match &error {
        ElicitationError::CapabilityNotSupported => format!(
            "{}已拒绝：客户端初始化时未声明 MCP form elicitation 能力，Astesia 无法向用户请求必要确认，因此没有执行该操作。请改用支持 elicitation 的 MCP 客户端。",
            match kind {
                ApprovalKind::Destructive => "高危操作",
                ApprovalKind::Update => "更新操作",
            }
        ),
        ElicitationError::UserDeclined => kind.declined_message().to_string(),
        ElicitationError::UserCancelled => kind.cancelled_message().to_string(),
        _ => kind.blocked_message(&error),
    };
    McpToolFailure::new(code, message, retryable, approval_details(kind))
}

pub(super) fn missing_approval_content(kind: ApprovalKind) -> McpToolFailure {
    McpToolFailure::new(
        ERROR_CODE_APPROVAL_INVALID_RESPONSE,
        kind.missing_content_message(),
        false,
        approval_details(kind),
    )
}

pub(super) fn declined_approval(kind: ApprovalKind) -> McpToolFailure {
    McpToolFailure::new(
        ERROR_CODE_APPROVAL_DECLINED,
        kind.declined_message(),
        false,
        approval_details(kind),
    )
}

impl AstesiaMcp {
    pub(super) async fn require_destructive_approval(
        &self,
        peer: &Peer<RoleServer>,
        message: String,
    ) -> Result<(), McpToolFailure> {
        let kind = ApprovalKind::Destructive;
        let response = peer
            .elicit_with_timeout::<DestructiveApproval>(message, Some(CONFIRMATION_TIMEOUT))
            .await
            .map_err(|error| map_elicitation_failure(kind, error))?
            .ok_or_else(|| missing_approval_content(kind))?;
        if response.confirm {
            Ok(())
        } else {
            Err(declined_approval(kind))
        }
    }

    pub(super) async fn require_update_approval(
        &self,
        peer: &Peer<RoleServer>,
        connection_id: &str,
        database: &str,
        message: String,
    ) -> Result<(), McpToolFailure> {
        let profile = self
            .catalog
            .profile(connection_id)
            .await
            .map_err(|error| match error {
                CatalogError::Repository(error) => McpToolFailure::from_repository(error),
                CatalogError::Message(message) => McpToolFailure::new(
                    ERROR_CODE_CONNECTION_NOT_FOUND,
                    message,
                    false,
                    json!({ "connection_id": connection_id }),
                ),
            })?;
        if self.catalog.updates_are_approved(&profile, database).await {
            return Ok(());
        }
        let kind = ApprovalKind::Update;
        let response = peer
            .elicit_with_timeout::<UpdateApproval>(message, Some(CONFIRMATION_TIMEOUT))
            .await
            .map_err(|error| map_elicitation_failure(kind, error))?
            .ok_or_else(|| missing_approval_content(kind))?;
        if !response.confirm {
            return Err(declined_approval(kind));
        }
        if response.do_not_ask_again {
            self.catalog.approve_updates(&profile, database).await;
        }
        Ok(())
    }

    pub(super) async fn approve_query_risk(
        &self,
        peer: &Peer<RoleServer>,
        query: &SavedQuery,
        analysis: &SqlAnalysis,
    ) -> Result<(), McpToolFailure> {
        if !analysis.requires_confirmation() {
            return Ok(());
        }
        let confirmation = analysis
            .confirmation_kind()
            .expect("confirmed risk must have a confirmation kind");
        let preview = format!(
            "连接: {}\n数据库: {}\n查询: {}\n风险: {}\nSQL:\n{}",
            query.connection_id,
            query.database,
            query.name,
            confirmation.as_str(),
            query.sql
        );
        if confirmation.allows_session_suppression() {
            self.require_update_approval(
                peer,
                &query.connection_id,
                &query.database,
                format!(
                    "即将执行 UPDATE。确认本次操作；可选择在当前 MCP 会话内不再提醒该连接的更新。\n\n{preview}"
                ),
            )
            .await
        } else {
            self.require_destructive_approval(
                peer,
                format!("即将执行不可自动豁免的高危 SQL，请逐项确认。\n\n{preview}"),
            )
            .await
        }
        .map_err(|failure| {
            failure.with_details(json!({
                "query_id": query.id,
                "connection_id": query.connection_id,
                "database": query.database,
                "risk": analysis.risk.as_str(),
                "classification_reason": match analysis.risk {
                    QueryRisk::Unknown => "Astesia 无法证明该 SQL 为只读；未列入安全白名单的函数调用或尚未建模的语法会按 fail-closed 规则要求确认。",
                    QueryRisk::Update => "SQL 会更新现有数据。",
                    QueryRisk::Delete => "SQL 会删除数据。",
                    QueryRisk::Permissions => "SQL 会修改用户、角色或权限。",
                    QueryRisk::Destructive => "SQL 包含不可逆或影响范围较大的结构变更。",
                    QueryRisk::ReadOnly | QueryRisk::Additive => "该风险等级不需要确认。",
                },
            }))
        })
    }
}
