use std::collections::HashSet;

use sqlx::{sqlite::SqliteRow, Row};

use super::{
    ConnectionRepositoryError, ConnectionRepositoryErrorCode, CredentialVerificationScope,
    SharedConnectionProfile, SharedConnectionRecord,
};
use crate::db::{ConnectionConfig, DbType};

pub(super) const SCHEMA_VERSION: i64 = 4;
pub(super) const MIN_SUPPORTED_SCHEMA_VERSION: i64 = 2;
pub(super) const STATE_TABLE: &str = "shared_connection_state";
pub(super) const PROFILES_TABLE: &str = "shared_connections";
pub(super) const META_TABLE: &str = "shared_connection_meta";
pub(super) const PENDING_CLEANUP_TABLE: &str = "pending_credential_cleanup";
pub(super) const CURRENT_SCHEMA_TABLES: [&str; 4] = [
    STATE_TABLE,
    PROFILES_TABLE,
    META_TABLE,
    PENDING_CLEANUP_TABLE,
];

const MAX_CONNECTION_ID_CHARS: usize = 256;
const MAX_NAME_CHARS: usize = 512;
const MAX_ENDPOINT_CHARS: usize = 4_096;
const MAX_USERNAME_CHARS: usize = 1_024;
const MAX_PASSWORD_BYTES: usize = 65_536;
const MAX_GROUP_NAME_CHARS: usize = 128;
const MAX_TAGS: usize = 20;
const MAX_TAG_CHARS: usize = 64;

const PROFILE_PROJECTION: &str = "id, name, db_type, host, port, username, database_name, color, \
    credential_ref, revision, mcp_enabled, group_name, tags_json";
const LEGACY_PROFILE_PROJECTION: &str = "id, name, db_type, host, port, username, database_name, \
    color, credential_ref, revision, mcp_enabled, NULL AS group_name, '[]' AS tags_json";

pub(super) fn profile_select(suffix: &str) -> String {
    format!("SELECT {PROFILE_PROJECTION} FROM {PROFILES_TABLE} {suffix}")
}

pub(super) fn profile_select_for_schema(schema_version: i64) -> String {
    let projection = if schema_version >= SCHEMA_VERSION {
        PROFILE_PROJECTION
    } else {
        LEGACY_PROFILE_PROJECTION
    };
    format!("SELECT {projection} FROM {PROFILES_TABLE}")
}

pub(super) fn required_tables_query() -> String {
    let names = CURRENT_SCHEMA_TABLES
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("SELECT name FROM sqlite_schema WHERE type = 'table' AND name IN ({names})")
}

pub(super) fn row_to_profile(
    row: &SqliteRow,
) -> Result<SharedConnectionProfile, ConnectionRepositoryError> {
    Ok(row_to_record(row)?.profile)
}

pub(super) fn profile_from_config(
    config: &ConnectionConfig,
    has_credential: bool,
    revision: i64,
    mcp_enabled: bool,
    group_name: Option<String>,
    tags: Vec<String>,
) -> SharedConnectionProfile {
    SharedConnectionProfile {
        id: config.id.clone(),
        name: config.name.clone(),
        db_type: config.db_type,
        host: config.host.clone(),
        port: config.port,
        username: config.username.clone(),
        database: config.database.clone(),
        color: config.color.clone(),
        group_name,
        tags,
        has_credential,
        revision,
        mcp_enabled,
    }
}

pub(super) fn row_to_record(
    row: &SqliteRow,
) -> Result<SharedConnectionRecord, ConnectionRepositoryError> {
    let db_type = db_type_from_str(row.try_get::<String, _>("db_type")?.as_str())?;
    let port = row.try_get::<i64, _>("port")?;
    let port = u16::try_from(port).map_err(|_| {
        ConnectionRepositoryError::new(
            ConnectionRepositoryErrorCode::StorageCorrupt,
            "共享连接仓库包含无效端口",
            "请从备份恢复仓库，或重新创建受影响的连接。",
        )
    })?;
    let credential_ref = row.try_get::<Option<String>, _>("credential_ref")?;
    let tags_json = row.try_get::<String, _>("tags_json")?;
    let tags = serde_json::from_str::<Vec<String>>(&tags_json).map_err(|_| {
        ConnectionRepositoryError::new(
            ConnectionRepositoryErrorCode::StorageCorrupt,
            "共享连接仓库包含无效标签数据",
            "请从备份恢复仓库，或重新保存受影响的连接。",
        )
    })?;
    Ok(SharedConnectionRecord {
        profile: SharedConnectionProfile {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            db_type,
            host: row.try_get("host")?,
            port,
            username: row.try_get("username")?,
            database: row.try_get("database_name")?,
            color: row.try_get("color")?,
            group_name: row.try_get("group_name")?,
            tags,
            has_credential: credential_ref.is_some(),
            revision: row.try_get("revision")?,
            mcp_enabled: row.try_get("mcp_enabled")?,
        },
        credential_ref,
    })
}

impl From<sqlx::Error> for ConnectionRepositoryError {
    fn from(error: sqlx::Error) -> Self {
        ConnectionRepositoryError::from_sqlx(error, "读取共享连接仓库")
    }
}

pub(super) fn validate_config(config: &ConnectionConfig) -> Result<(), ConnectionRepositoryError> {
    let id = config.id.trim();
    if id.is_empty() || id.chars().count() > MAX_CONNECTION_ID_CHARS {
        return Err(ConnectionRepositoryError::invalid(format!(
            "connection_id 必须为 1-{MAX_CONNECTION_ID_CHARS} 个字符"
        )));
    }
    if config.name.trim().is_empty() || config.name.chars().count() > MAX_NAME_CHARS {
        return Err(ConnectionRepositoryError::invalid(format!(
            "连接名称必须为 1-{MAX_NAME_CHARS} 个字符"
        )));
    }
    if config.host.chars().count() > MAX_ENDPOINT_CHARS || config.host.chars().any(char::is_control)
    {
        return Err(ConnectionRepositoryError::invalid(
            "host/SQLite 路径过长或包含控制字符",
        ));
    }
    if config.username.chars().count() > MAX_USERNAME_CHARS
        || config.username.chars().any(char::is_control)
    {
        return Err(ConnectionRepositoryError::invalid(
            "username 过长或包含控制字符",
        ));
    }
    if config.password.len() > MAX_PASSWORD_BYTES {
        return Err(ConnectionRepositoryError::invalid(format!(
            "密码不能超过 {MAX_PASSWORD_BYTES} 字节"
        )));
    }
    if config
        .database
        .as_deref()
        .is_some_and(|value| value.chars().any(char::is_control))
    {
        return Err(ConnectionRepositoryError::invalid(
            "database 不能包含控制字符",
        ));
    }
    Ok(())
}

pub(super) fn normalize_group_name(
    group_name: Option<String>,
) -> Result<Option<String>, ConnectionRepositoryError> {
    let Some(group_name) = group_name else {
        return Ok(None);
    };
    let group_name = group_name.trim();
    if group_name.is_empty() {
        return Ok(None);
    }
    if group_name.chars().count() > MAX_GROUP_NAME_CHARS || group_name.chars().any(char::is_control)
    {
        return Err(ConnectionRepositoryError::invalid(format!(
            "分组名称不能超过 {MAX_GROUP_NAME_CHARS} 个字符，且不能包含控制字符"
        )));
    }
    Ok(Some(group_name.to_string()))
}

pub(super) fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>, ConnectionRepositoryError> {
    let mut normalized = Vec::with_capacity(tags.len());
    let mut seen = HashSet::with_capacity(tags.len());
    for tag in tags {
        let tag = tag.trim();
        if tag.is_empty() {
            continue;
        }
        if tag.chars().count() > MAX_TAG_CHARS || tag.chars().any(char::is_control) {
            return Err(ConnectionRepositoryError::invalid(format!(
                "标签不能超过 {MAX_TAG_CHARS} 个字符，且不能包含控制字符"
            )));
        }
        let key = tag.to_lowercase();
        if seen.insert(key) {
            normalized.push(tag.to_string());
            if normalized.len() > MAX_TAGS {
                return Err(ConnectionRepositoryError::invalid(format!(
                    "每个连接最多可设置 {MAX_TAGS} 个标签"
                )));
            }
        }
    }
    Ok(normalized)
}

pub(super) fn serialize_tags(tags: &[String]) -> Result<String, ConnectionRepositoryError> {
    serde_json::to_string(tags).map_err(|_| {
        ConnectionRepositoryError::new(
            ConnectionRepositoryErrorCode::StorageUnavailable,
            "无法序列化连接标签",
            "请检查标签内容后重试。",
        )
    })
}

pub(super) fn validate_unique_configs(
    configs: &[ConnectionConfig],
) -> Result<(), ConnectionRepositoryError> {
    let mut ids = HashSet::new();
    for config in configs {
        if !ids.insert(config.id.as_str()) {
            return Err(ConnectionRepositoryError::invalid(format!(
                "旧连接数据包含重复 connection_id：{}",
                config.id
            )));
        }
    }
    Ok(())
}

/// Builds versioned, unambiguous AEAD associated data for a credential.
///
/// Display-only fields are intentionally excluded so a rename or color change
/// can retain the credential. Endpoint and account fields are included so an
/// edited repository cannot redirect a valid ciphertext to another target.
pub(super) fn credential_binding(config: &ConnectionConfig) -> Vec<u8> {
    let mut binding = b"astesia.connection-credential.v1".to_vec();
    append_binding_field(&mut binding, config.id.as_bytes());
    append_binding_field(&mut binding, db_type_to_str(&config.db_type).as_bytes());
    append_binding_field(&mut binding, config.host.as_bytes());
    append_binding_field(&mut binding, &config.port.to_be_bytes());
    append_binding_field(&mut binding, config.username.as_bytes());
    match config.database.as_deref() {
        Some(database) => {
            binding.push(1);
            append_binding_field(&mut binding, database.as_bytes());
        }
        None => binding.push(0),
    }
    binding
}

fn append_binding_field(binding: &mut Vec<u8>, value: &[u8]) {
    binding.extend_from_slice(&(value.len() as u64).to_be_bytes());
    binding.extend_from_slice(value);
}

pub(super) fn credential_scope(
    repository_id: &str,
    repository_revision: i64,
    profiles: &[SharedConnectionProfile],
) -> CredentialVerificationScope {
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    digest.update(b"astesia.credential-verification-scope.v3");
    let mut sorted_profiles = profiles.iter().collect::<Vec<_>>();
    sorted_profiles.sort_by(|left, right| left.id.cmp(&right.id));
    for profile in &sorted_profiles {
        update_digest_field(&mut digest, profile.id.as_bytes());
        update_digest_field(&mut digest, profile.name.as_bytes());
        update_digest_field(&mut digest, db_type_to_str(&profile.db_type).as_bytes());
        update_digest_field(&mut digest, profile.host.as_bytes());
        update_digest_field(&mut digest, &profile.port.to_be_bytes());
        update_digest_field(&mut digest, profile.username.as_bytes());
        update_digest_optional_field(&mut digest, profile.database.as_deref());
        update_digest_optional_field(&mut digest, profile.color.as_deref());
        update_digest_optional_field(&mut digest, profile.group_name.as_deref());
        update_digest_field(&mut digest, &(profile.tags.len() as u64).to_be_bytes());
        for tag in &profile.tags {
            update_digest_field(&mut digest, tag.as_bytes());
        }
        update_digest_field(&mut digest, &[u8::from(profile.has_credential)]);
        update_digest_field(&mut digest, &profile.revision.to_be_bytes());
        update_digest_field(&mut digest, &[u8::from(profile.mcp_enabled)]);
    }
    let digest = digest.finish();
    let mut profile_digest = String::with_capacity(digest.as_ref().len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest.as_ref() {
        profile_digest.push(HEX[usize::from(byte >> 4)] as char);
        profile_digest.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    CredentialVerificationScope {
        repository_id: repository_id.to_string(),
        repository_revision,
        profile_count: sorted_profiles.len(),
        credential_count: sorted_profiles
            .iter()
            .filter(|profile| profile.has_credential)
            .count(),
        profile_digest,
    }
}

fn update_digest_field(digest: &mut ring::digest::Context, value: &[u8]) {
    digest.update(&(value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn update_digest_optional_field(digest: &mut ring::digest::Context, value: Option<&str>) {
    match value {
        Some(value) => {
            digest.update(&[1]);
            update_digest_field(digest, value.as_bytes());
        }
        None => digest.update(&[0]),
    }
}

pub(super) fn db_type_to_str(db_type: &DbType) -> &'static str {
    match db_type {
        DbType::MySQL => "mysql",
        DbType::PostgreSQL => "postgresql",
        DbType::SQLite => "sqlite",
        DbType::SQLServer => "sqlserver",
        DbType::MongoDB => "mongodb",
        DbType::Redis => "redis",
        DbType::ClickHouse => "clickhouse",
    }
}

fn db_type_from_str(value: &str) -> Result<DbType, ConnectionRepositoryError> {
    match value {
        "mysql" => Ok(DbType::MySQL),
        "postgresql" => Ok(DbType::PostgreSQL),
        "sqlite" => Ok(DbType::SQLite),
        "sqlserver" => Ok(DbType::SQLServer),
        "mongodb" => Ok(DbType::MongoDB),
        "redis" => Ok(DbType::Redis),
        "clickhouse" => Ok(DbType::ClickHouse),
        _ => Err(ConnectionRepositoryError::new(
            ConnectionRepositoryErrorCode::StorageCorrupt,
            format!("共享连接仓库包含未知数据库类型：{value}"),
            "请从备份恢复仓库，或重新创建受影响的连接。",
        )),
    }
}

pub(super) fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database_error) if database_error.is_unique_violation()
    )
}

pub(super) fn is_sqlite_busy_error(error: &sqlx::Error) -> bool {
    let sqlx::Error::Database(database_error) = error else {
        return false;
    };
    database_error.code().is_some_and(|code| {
        code.parse::<i32>()
            .is_ok_and(|code| matches!(code & 0xff, 5 | 6))
    })
}
