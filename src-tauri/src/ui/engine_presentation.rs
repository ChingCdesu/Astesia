use gpui::{rgb, Rgba};

use crate::connection_repository::SharedConnectionProfile;
use crate::db::DbType;

pub(super) const fn engine_label(db_type: DbType) -> &'static str {
    match db_type {
        DbType::MySQL => "MySQL",
        DbType::PostgreSQL => "PostgreSQL",
        DbType::SQLite => "SQLite",
        DbType::SQLServer => "SQL Server",
        DbType::MongoDB => "MongoDB",
        DbType::Redis => "Redis",
        DbType::ClickHouse => "ClickHouse",
    }
}

pub(super) const fn engine_hex_color(db_type: DbType) -> &'static str {
    match db_type {
        DbType::MySQL => "#00758F",
        DbType::PostgreSQL => "#336791",
        DbType::SQLite => "#003B57",
        DbType::SQLServer => "#CC2927",
        DbType::MongoDB => "#47A248",
        DbType::Redis => "#DC382D",
        DbType::ClickHouse => "#FFCC01",
    }
}

pub(super) fn engine_color(db_type: DbType) -> Rgba {
    rgb(match db_type {
        DbType::MySQL => 0x00758f,
        DbType::PostgreSQL => 0x336791,
        DbType::SQLite => 0x003b57,
        DbType::SQLServer => 0xcc2927,
        DbType::MongoDB => 0x47a248,
        DbType::Redis => 0xdc382d,
        DbType::ClickHouse => 0xffcc01,
    })
}

pub(super) fn profile_color(profile: &SharedConnectionProfile) -> Rgba {
    profile
        .color
        .as_deref()
        .and_then(parse_hex_color)
        .unwrap_or_else(|| engine_color(profile.db_type))
}

pub(super) fn profile_endpoint(profile: &SharedConnectionProfile) -> String {
    if profile.db_type.profile_spec().is_file() || profile.port == 0 {
        profile.host.clone()
    } else {
        format!("{}:{}", profile.host, profile.port)
    }
}

fn parse_hex_color(value: &str) -> Option<Rgba> {
    let value = value.strip_prefix('#').unwrap_or(value);
    (value.len() == 6)
        .then(|| u32::from_str_radix(value, 16).ok().map(rgb))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, db_type: DbType) -> SharedConnectionProfile {
        SharedConnectionProfile {
            id: id.to_string(),
            name: id.to_string(),
            db_type,
            host: if db_type == DbType::SQLite {
                "/tmp/astesia.sqlite3".to_string()
            } else {
                "127.0.0.1".to_string()
            },
            port: db_type.profile_spec().default_port(),
            username: "tester".to_string(),
            database: None,
            color: None,
            group_name: None,
            tags: Vec::new(),
            has_credential: false,
            revision: 1,
            mcp_enabled: false,
        }
    }

    #[test]
    fn endpoint_and_color_formatters_preserve_profile_identity() {
        let sqlite = profile("sqlite", DbType::SQLite);
        let mut postgres = profile("postgres", DbType::PostgreSQL);
        postgres.color = Some("#abcdef".to_string());

        assert_eq!(profile_endpoint(&sqlite), "/tmp/astesia.sqlite3");
        assert_eq!(profile_endpoint(&postgres), "127.0.0.1:5432");
        assert_eq!(profile_color(&postgres), rgb(0xabcdef));
        assert_eq!(parse_hex_color("not-a-color"), None);
    }
}
