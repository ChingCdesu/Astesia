use std::{fmt, sync::Arc};

use async_trait::async_trait;
use serde::Serialize;

mod envelope;
mod platform;
mod system;

#[cfg(test)]
pub(crate) mod test_support;

pub use system::SystemCredentialVault;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialVaultErrorCode {
    Missing,
    MigrationRequired,
    StoreUnavailable,
    AccessDenied,
    Corrupt,
    Invalid,
}

impl CredentialVaultErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "credential_missing",
            Self::MigrationRequired => "credential_migration_required",
            Self::StoreUnavailable => "credential_store_unavailable",
            Self::AccessDenied => "credential_access_denied",
            Self::Corrupt => "credential_corrupt",
            Self::Invalid => "credential_invalid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialVaultError {
    pub code: CredentialVaultErrorCode,
    pub message: String,
    pub remediation: String,
}

impl CredentialVaultError {
    pub(super) fn new(code: CredentialVaultErrorCode, operation: &str) -> Self {
        let (message, remediation) = match code {
            CredentialVaultErrorCode::Missing => (
                format!("无法{operation}：该连接没有可用的已保存凭据"),
                "请在 Astesia App 中重新输入并保存该连接的密码。".to_string(),
            ),
            CredentialVaultErrorCode::MigrationRequired => (
                format!("无法{operation}：旧版凭据尚未完成安全迁移"),
                "请先打开 Astesia App 并完成强制凭据迁移，然后重新启动 MCP 客户端。STDIO 不会直接读取或删除旧版系统钥匙串条目。"
                    .to_string(),
            ),
            CredentialVaultErrorCode::StoreUnavailable => (
                format!("无法{operation}：操作系统凭据库不可用"),
                platform::remediation(),
            ),
            CredentialVaultErrorCode::AccessDenied => (
                format!("无法{operation}：操作系统凭据库已锁定或拒绝访问"),
                "请解锁当前用户的系统凭据库、允许 Astesia 访问后重试。".to_string(),
            ),
            CredentialVaultErrorCode::Corrupt => (
                format!("无法{operation}：系统凭据库中的数据损坏或格式不受支持"),
                "请在 Astesia App 中重新保存该连接的密码。".to_string(),
            ),
            CredentialVaultErrorCode::Invalid => (
                format!("无法{operation}：凭据标识或内容不符合系统凭据库要求"),
                "请检查连接标识和密码长度后重试。".to_string(),
            ),
        };
        Self {
            code,
            message,
            remediation,
        }
    }

    pub(super) fn task_failed(operation: &str) -> Self {
        Self::new(CredentialVaultErrorCode::StoreUnavailable, operation)
    }

    pub(super) fn master_missing(operation: &str) -> Self {
        Self {
            code: CredentialVaultErrorCode::Corrupt,
            message: format!(
                "无法{operation}：加密凭据保险库已存在，但操作系统凭据库中的主密钥缺失"
            ),
            remediation:
                "Astesia 已拒绝生成替代主密钥，以免覆盖现有密文。请恢复系统凭据库中的 Astesia 主密钥，或在 App 中删除无法解密的连接后重新保存凭据。"
                    .to_string(),
        }
    }

    pub(super) fn master_changed(operation: &str) -> Self {
        Self {
            code: CredentialVaultErrorCode::Corrupt,
            message: format!("无法{operation}：操作系统凭据库中的主密钥与当前加密凭据保险库不匹配"),
            remediation:
                "Astesia 已拒绝写入新密文。请关闭其他 Astesia 进程并恢复正确的系统主密钥后重试。"
                    .to_string(),
        }
    }
}

impl fmt::Display for CredentialVaultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}（错误码：{}）。{}",
            self.message,
            self.code.as_str(),
            self.remediation
        )
    }
}

impl std::error::Error for CredentialVaultError {}

#[async_trait]
pub trait CredentialVault: Send + Sync {
    async fn put(&self, binding: &[u8], secret: &str) -> Result<String, CredentialVaultError>;
    async fn get(&self, reference: &str, binding: &[u8]) -> Result<String, CredentialVaultError>;
    async fn delete(&self, reference: &str) -> Result<(), CredentialVaultError>;
}

pub type CredentialVaultHandle = Arc<dyn CredentialVault>;
