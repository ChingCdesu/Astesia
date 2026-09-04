use crate::application::{ConnectionProfileSnapshot, ConnectionWorkspaceSnapshot};
use crate::connection_repository::ConnectionRepositoryError;

pub(super) struct ProfileGroup<'a> {
    pub(super) name: Option<&'a str>,
    pub(super) profiles: Vec<&'a ConnectionProfileSnapshot>,
}

pub(super) fn grouped_profiles(snapshot: &ConnectionWorkspaceSnapshot) -> Vec<ProfileGroup<'_>> {
    let mut named_groups: Vec<ProfileGroup<'_>> = Vec::new();
    let mut ungrouped = Vec::new();

    for profile in &snapshot.profiles {
        let group_name = profile
            .profile
            .group_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty());
        if let Some(group_name) = group_name {
            if let Some(group) = named_groups
                .iter_mut()
                .find(|group| group.name == Some(group_name))
            {
                group.profiles.push(profile);
            } else {
                named_groups.push(ProfileGroup {
                    name: Some(group_name),
                    profiles: vec![profile],
                });
            }
        } else {
            ungrouped.push(profile);
        }
    }

    named_groups.sort_by(|left, right| {
        left.name
            .unwrap_or_default()
            .to_lowercase()
            .cmp(&right.name.unwrap_or_default().to_lowercase())
    });
    if !ungrouped.is_empty() {
        named_groups.push(ProfileGroup {
            name: None,
            profiles: ungrouped,
        });
    }
    named_groups
}

pub(super) fn repository_error_message(error: &ConnectionRepositoryError) -> String {
    format!("{} {}", error.message, error.remediation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::{ConnectionProfileSnapshot, DatabaseSessionSnapshot};
    use crate::connection_repository::SharedConnectionProfile;
    use crate::db::DbType;

    fn profile(id: &str, group_name: Option<&str>, db_type: DbType) -> SharedConnectionProfile {
        SharedConnectionProfile {
            id: id.to_string(),
            name: id.to_string(),
            db_type,
            host: "127.0.0.1".to_string(),
            port: db_type.profile_spec().default_port(),
            username: "tester".to_string(),
            database: None,
            color: None,
            group_name: group_name.map(str::to_string),
            tags: Vec::new(),
            has_credential: false,
            revision: 1,
            mcp_enabled: false,
        }
    }

    #[test]
    fn named_groups_sort_before_ungrouped_profiles() {
        let snapshot = ConnectionWorkspaceSnapshot {
            repository_revision: 1,
            mcp_revision: 0,
            profiles: [
                profile("loose", None, DbType::SQLite),
                profile("zeta", Some("Zeta"), DbType::Redis),
                profile("alpha", Some("Alpha"), DbType::MySQL),
            ]
            .into_iter()
            .map(|profile| ConnectionProfileSnapshot {
                profile,
                session: DatabaseSessionSnapshot { generation: None },
                mcp_usage: None,
            })
            .collect(),
        };
        let groups = grouped_profiles(&snapshot);

        assert_eq!(
            groups.iter().map(|group| group.name).collect::<Vec<_>>(),
            vec![Some("Alpha"), Some("Zeta"), None]
        );
        assert_eq!(groups[2].profiles[0].profile.id, "loose");
    }
}
