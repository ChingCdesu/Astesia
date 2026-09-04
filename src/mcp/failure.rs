use rmcp::model::CallToolResult;
use serde_json::{json, Value};

use crate::connection_repository::ConnectionRepositoryError;

use super::{catalog::CatalogError, sql};

pub(super) const ERROR_CODE_TOOL_FAILED: &str = "astesia.tool.failed";
pub(super) const ERROR_CODE_CONNECTION_NOT_FOUND: &str = "astesia.connection.not_found";
pub(super) const ERROR_CODE_APPROVAL_UNSUPPORTED: &str = "astesia.approval.unsupported";
pub(super) const ERROR_CODE_APPROVAL_DECLINED: &str = "astesia.approval.declined";
pub(super) const ERROR_CODE_APPROVAL_CANCELLED: &str = "astesia.approval.cancelled";
pub(super) const ERROR_CODE_APPROVAL_TIMEOUT: &str = "astesia.approval.timeout";
pub(super) const ERROR_CODE_APPROVAL_INVALID_RESPONSE: &str = "astesia.approval.invalid_response";
pub(super) const ERROR_CODE_APPROVAL_UNAVAILABLE: &str = "astesia.approval.unavailable";

#[derive(Debug, Clone, PartialEq)]
pub(super) struct McpToolFailure {
    pub(super) code: String,
    pub(super) message: String,
    pub(super) remediation: Option<String>,
    pub(super) retryable: bool,
    pub(super) details: Box<Value>,
}

impl McpToolFailure {
    pub(super) fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
        details: Value,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            remediation: None,
            retryable,
            details: Box::new(details),
        }
    }

    pub(super) fn from_repository(error: ConnectionRepositoryError) -> Self {
        Self {
            code: error.code.as_str().to_string(),
            message: error.message,
            remediation: Some(error.remediation),
            retryable: error.retryable,
            details: error.details,
        }
    }

    pub(super) fn with_details(mut self, details: Value) -> Self {
        if let (Some(current), Some(additional)) =
            (self.details.as_object_mut(), details.as_object())
        {
            current.extend(additional.clone());
        } else {
            self.details = Box::new(details);
        }
        self
    }
}

impl From<McpToolFailure> for CallToolResult {
    fn from(failure: McpToolFailure) -> Self {
        let mut payload = json!({
            "ok": false,
            "error": failure.message,
            "error_code": failure.code,
            "retryable": failure.retryable,
            "details": failure.details,
        });
        if let Some(remediation) = failure.remediation {
            payload["remediation"] = Value::String(remediation);
        }
        CallToolResult::structured_error(payload)
    }
}

pub(super) enum McpFailureSource {
    Message(String),
    Repository(ConnectionRepositoryError),
}

impl From<String> for McpFailureSource {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl From<&str> for McpFailureSource {
    fn from(message: &str) -> Self {
        Self::Message(message.to_string())
    }
}

impl From<CatalogError> for McpFailureSource {
    fn from(error: CatalogError) -> Self {
        match error {
            CatalogError::Repository(error) => Self::Repository(error),
            CatalogError::Message(message) => Self::Message(message),
        }
    }
}

impl From<sql::SqlBuildError> for McpFailureSource {
    fn from(error: sql::SqlBuildError) -> Self {
        Self::Message(error.to_string())
    }
}
