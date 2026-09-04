use super::*;
use crate::application::{
    ConnectionProfileSnapshot, ConnectionWorkspaceSnapshot, DatabaseSessionSnapshot,
};
use crate::db::DbType;

fn profile(id: &str) -> SharedConnectionProfile {
    SharedConnectionProfile {
        id: id.to_string(),
        name: id.to_string(),
        db_type: DbType::PostgreSQL,
        host: "127.0.0.1".to_string(),
        port: 5432,
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

fn snapshot(profile: SharedConnectionProfile) -> ConnectionWorkspaceSnapshot {
    ConnectionWorkspaceSnapshot {
        repository_revision: 1,
        mcp_revision: 0,
        profiles: vec![ConnectionProfileSnapshot {
            profile,
            session: DatabaseSessionSnapshot {
                generation: Some(7),
            },
            mcp_usage: None,
        }],
    }
}

#[test]
fn replacing_profiles_clears_a_missing_selection() {
    let mut selected = Some("primary".to_string());
    let empty = ConnectionWorkspaceSnapshot {
        repository_revision: 2,
        mcp_revision: 0,
        profiles: Vec::new(),
    };

    reconcile_selected_profile(&mut selected, Some(&empty));

    assert!(selected.is_none());
}

#[test]
fn structured_status_tracks_selected_session_and_operation() {
    let mut state = ConnectionWorkspaceState::default();
    let refresh = state.begin_refresh();
    state.finish_refresh(refresh, Ok(snapshot(profile("primary"))));

    let status = derive_status(
        &state,
        Some("primary"),
        None,
        false,
        crate::platform::UiLanguage::Chinese,
    );
    assert_eq!(status.session, ConnectionSessionStatus::Connected);
    assert_eq!(status.activity, ConnectionActivityStatus::Ready);

    let status = derive_status(
        &state,
        Some("primary"),
        None,
        true,
        crate::platform::UiLanguage::Chinese,
    );
    assert_eq!(status.activity, ConnectionActivityStatus::Working);

    state.begin_operation("primary", ProfileOperationKind::Disconnecting);
    let status = derive_status(
        &state,
        Some("primary"),
        None,
        false,
        crate::platform::UiLanguage::Chinese,
    );
    assert_eq!(status.session, ConnectionSessionStatus::Disconnecting);
    assert_eq!(status.activity, ConnectionActivityStatus::Working);
}
