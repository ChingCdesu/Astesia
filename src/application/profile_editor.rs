use std::collections::HashSet;

use crate::{
    connection_repository::{SaveConnectionRequest, SharedConnectionProfile},
    db::{ConnectionConfig, DbType, EngineProfileSpec},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProfileOrigin {
    Create {
        profile_id: String,
    },
    Edit {
        profile_id: String,
        expected_revision: i64,
        mcp_enabled: bool,
        had_credential: bool,
    },
}

impl ProfileOrigin {
    pub(crate) fn create(profile_id: String) -> Self {
        Self::Create { profile_id }
    }

    pub(crate) fn edit(profile: &SharedConnectionProfile) -> Self {
        Self::Edit {
            profile_id: profile.id.clone(),
            expected_revision: profile.revision,
            mcp_enabled: profile.mcp_enabled,
            had_credential: profile.has_credential,
        }
    }

    pub(crate) fn is_editing(&self) -> bool {
        matches!(self, Self::Edit { .. })
    }

    pub(crate) fn removes_saved_credential(&self, db_type: DbType) -> bool {
        matches!(
            self,
            Self::Edit {
                had_credential: true,
                ..
            }
        ) && db_type.profile_spec().is_file()
    }

    fn profile_id(&self) -> &str {
        match self {
            Self::Create { profile_id } | Self::Edit { profile_id, .. } => profile_id,
        }
    }

    fn expected_revision(&self) -> Option<i64> {
        match self {
            Self::Create { .. } => None,
            Self::Edit {
                expected_revision, ..
            } => Some(*expected_revision),
        }
    }

    fn mcp_enabled(&self) -> bool {
        match self {
            Self::Create { .. } => true,
            Self::Edit { mcp_enabled, .. } => *mcp_enabled,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProfileDraftField {
    Name,
    Endpoint,
    Port,
    Tags,
    Color,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProfileValidationError {
    NameRequired,
    FilePathRequired,
    HostRequired,
    InvalidPort,
    InvalidColor,
    InvalidTag,
    TooManyTags,
}

impl ProfileValidationError {
    pub(crate) fn field(self) -> ProfileDraftField {
        match self {
            Self::NameRequired => ProfileDraftField::Name,
            Self::FilePathRequired | Self::HostRequired => ProfileDraftField::Endpoint,
            Self::InvalidPort => ProfileDraftField::Port,
            Self::InvalidColor => ProfileDraftField::Color,
            Self::InvalidTag | Self::TooManyTags => ProfileDraftField::Tags,
        }
    }
}

pub(crate) struct ProfileDraft {
    pub(crate) db_type: DbType,
    pub(crate) name: String,
    pub(crate) endpoint: String,
    pub(crate) port: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) database: String,
    pub(crate) group_name: String,
    pub(crate) tags: String,
    pub(crate) color: String,
}

pub(crate) struct ValidatedProfile {
    request: SaveConnectionRequest,
}

impl ValidatedProfile {
    pub(crate) fn config(&self) -> &ConnectionConfig {
        &self.request.config
    }

    pub(in crate::application) fn into_request(self) -> SaveConnectionRequest {
        self.request
    }

    #[cfg(test)]
    pub(in crate::application) fn from_request(request: SaveConnectionRequest) -> Self {
        Self { request }
    }

    #[cfg(test)]
    pub(in crate::application) fn request_mut(&mut self) -> &mut SaveConnectionRequest {
        &mut self.request
    }
}

impl ProfileDraft {
    pub(crate) fn validate(
        self,
        origin: &ProfileOrigin,
    ) -> Result<ValidatedProfile, Vec<ProfileValidationError>> {
        let spec = self.db_type.profile_spec();
        let name = self.name.trim().to_string();
        let endpoint = self.endpoint.trim().to_string();
        let mut errors = Vec::new();

        if name.is_empty() {
            errors.push(ProfileValidationError::NameRequired);
        }
        if endpoint.is_empty() {
            errors.push(match spec {
                EngineProfileSpec::File { .. } => ProfileValidationError::FilePathRequired,
                EngineProfileSpec::Network { .. } => ProfileValidationError::HostRequired,
            });
        }

        let (port, username, password, database) = match spec {
            EngineProfileSpec::File { .. } => (0, String::new(), String::new(), None),
            EngineProfileSpec::Network { default_port, .. } => {
                let port_text = self.port.trim();
                let port = if port_text.is_empty() {
                    default_port
                } else {
                    match port_text.parse::<u16>() {
                        Ok(port) if port > 0 => port,
                        _ => {
                            errors.push(ProfileValidationError::InvalidPort);
                            0
                        }
                    }
                };
                (
                    port,
                    self.username.trim().to_string(),
                    self.password,
                    optional_text(self.database),
                )
            }
        };

        let tags = match normalize_tags(&self.tags) {
            Ok(tags) => tags,
            Err(error) => {
                errors.push(error);
                Vec::new()
            }
        };
        let color = match normalize_color(&self.color) {
            Ok(color) => color,
            Err(error) => {
                errors.push(error);
                None
            }
        };

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(ValidatedProfile {
            request: SaveConnectionRequest {
                config: ConnectionConfig {
                    id: origin.profile_id().to_string(),
                    name,
                    db_type: self.db_type,
                    host: endpoint,
                    port,
                    username,
                    password,
                    database,
                    color,
                },
                expected_revision: origin.expected_revision(),
                mcp_enabled: origin.mcp_enabled(),
                group_name: optional_text(self.group_name),
                tags,
            },
        })
    }
}

fn optional_text(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn normalize_color(value: &str) -> Result<Option<String>, ProfileValidationError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 || !hex.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err(ProfileValidationError::InvalidColor);
    }
    Ok(Some(format!("#{}", hex.to_ascii_uppercase())))
}

fn normalize_tags(value: &str) -> Result<Vec<String>, ProfileValidationError> {
    let mut tags = Vec::new();
    let mut seen = HashSet::new();
    for tag in value
        .split([',', '，', '\n'])
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
    {
        if tag.chars().count() > 64 || tag.chars().any(char::is_control) {
            return Err(ProfileValidationError::InvalidTag);
        }
        if seen.insert(tag.to_lowercase()) {
            tags.push(tag.to_string());
            if tags.len() > 20 {
                return Err(ProfileValidationError::TooManyTags);
            }
        }
    }
    Ok(tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_tags_and_color() {
        assert_eq!(
            normalize_tags(" prod, 重要，PROD ").unwrap(),
            vec!["prod", "重要"]
        );
        assert_eq!(
            normalize_color("336791").unwrap(),
            Some("#336791".to_string())
        );
        assert!(normalize_color("#xyz").is_err());
    }

    #[test]
    fn file_engine_discards_network_only_values() {
        let origin = ProfileOrigin::create("sqlite".to_string());
        let profile = ProfileDraft {
            db_type: DbType::SQLite,
            name: "Local".to_string(),
            endpoint: "/tmp/astesia.sqlite3".to_string(),
            port: "5432".to_string(),
            username: "ignored".to_string(),
            password: "ignored".to_string(),
            database: "ignored".to_string(),
            group_name: String::new(),
            tags: String::new(),
            color: String::new(),
        }
        .validate(&origin)
        .expect("draft is valid");

        assert_eq!(profile.config().port, 0);
        assert!(profile.config().username.is_empty());
        assert!(profile.config().password.is_empty());
        assert!(profile.config().database.is_none());
    }

    #[test]
    fn edit_origin_preserves_revision_and_credential_policy() {
        let origin = ProfileOrigin::Edit {
            profile_id: "primary".to_string(),
            expected_revision: 4,
            mcp_enabled: false,
            had_credential: true,
        };

        assert_eq!(origin.expected_revision(), Some(4));
        assert!(!origin.mcp_enabled());
        assert!(origin.removes_saved_credential(DbType::SQLite));
        assert!(!origin.removes_saved_credential(DbType::PostgreSQL));
    }

    #[test]
    fn every_supported_engine_accepts_its_native_profile_defaults() {
        for db_type in DbType::all() {
            let spec = db_type.profile_spec();
            let origin = ProfileOrigin::create(format!("{db_type:?}"));
            let draft = ProfileDraft {
                db_type,
                name: format!("{db_type:?}"),
                endpoint: if spec.is_file() {
                    ":memory:".to_string()
                } else {
                    spec.default_endpoint().to_string()
                },
                port: spec.default_port().to_string(),
                username: spec.default_username().to_string(),
                password: "disposable-test-password".to_string(),
                database: spec.default_database().unwrap_or_default().to_string(),
                group_name: "Milestone 3".to_string(),
                tags: "native, smoke".to_string(),
                color: String::new(),
            };

            let profile = draft
                .validate(&origin)
                .unwrap_or_else(|errors| panic!("{db_type:?} defaults failed: {errors:?}"));

            assert_eq!(profile.config().db_type, db_type);
            assert_eq!(profile.request.group_name.as_deref(), Some("Milestone 3"));
            assert_eq!(profile.request.tags, ["native", "smoke"]);
        }
    }
}
