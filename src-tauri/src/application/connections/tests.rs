use super::*;
use crate::{
    application::QueryService,
    credential_vault::test_support::MemoryCredentialVault,
    db::{ConnectionConfig, DbType},
};

fn sqlite_config(id: &str) -> ConnectionConfig {
    ConnectionConfig {
        id: id.to_string(),
        name: id.to_string(),
        db_type: DbType::SQLite,
        host: ":memory:".to_string(),
        port: 0,
        username: String::new(),
        password: String::new(),
        database: None,
        color: None,
    }
}

fn manager() -> (tempfile::TempDir, ConnectionManager) {
    let directory = tempfile::TempDir::new().expect("tempdir");
    let repository = SharedConnectionRepository::new(
        directory.path().join("connections.sqlite3"),
        MemoryCredentialVault::shared(),
    );
    (directory, ConnectionManager::new(repository))
}

async fn is_connected(manager: &ConnectionManager, connection_id: &str) -> bool {
    manager.runtime.contains(connection_id).await
}

#[tokio::test]
async fn disconnect_invalidates_an_in_flight_connect_without_an_installed_driver() {
    let (_directory, manager) = manager();
    let generation = manager.begin_connect_intent("local").await;

    assert!(!manager.disconnect_local("local").await);

    let installation = manager
        .runtime
        .connect_replacing(
            "local".to_string(),
            generation,
            sqlite_config("late"),
            1,
            (),
            || async { Ok::<_, ()>(1) },
        )
        .await;
    assert!(matches!(
        installation,
        Err(ReplacingConnectError::Superseded)
    ));
    assert!(!is_connected(&manager, "local").await);
}

#[tokio::test]
async fn successful_save_and_delete_invalidate_pending_connect_intents() {
    let (_directory, manager) = manager();
    let saved = manager
        .repository
        .create(sqlite_config("saved"), false)
        .await
        .expect("create saved profile");
    let deleted = manager
        .repository
        .create(sqlite_config("deleted"), false)
        .await
        .expect("create deleted profile");
    let save_generation = manager.begin_connect_intent("saved").await;
    let delete_generation = manager.begin_connect_intent("deleted").await;

    let mut updated = sqlite_config("saved");
    updated.name = "Updated".to_string();
    manager
        .save_profile(SaveConnectionRequest {
            config: updated,
            expected_revision: Some(saved.revision),
            mcp_enabled: false,
            group_name: None,
            tags: Vec::new(),
        })
        .await
        .expect("save profile");
    manager
        .delete_profile("deleted", deleted.revision)
        .await
        .expect("delete profile");

    let after_save = manager
        .runtime
        .connect_replacing(
            "saved".to_string(),
            save_generation,
            sqlite_config("after-save"),
            saved.revision,
            (),
            || async { Ok::<_, ()>(saved.revision) },
        )
        .await;
    let after_delete = manager
        .runtime
        .connect_replacing(
            "deleted".to_string(),
            delete_generation,
            sqlite_config("after-delete"),
            deleted.revision,
            (),
            || async { Ok::<_, ()>(deleted.revision) },
        )
        .await;
    assert!(matches!(after_save, Err(ReplacingConnectError::Superseded)));
    assert!(matches!(
        after_delete,
        Err(ReplacingConnectError::Superseded)
    ));
    assert!(!is_connected(&manager, "saved").await);
    assert!(!is_connected(&manager, "deleted").await);
}

#[tokio::test]
async fn session_connect_query_and_disconnect_cross_the_application_interface() {
    let (_directory, manager) = manager();
    manager
        .repository
        .create(sqlite_config("local"), false)
        .await
        .expect("create profile");

    let connected = manager.connect("local").await.expect("connect");
    assert_eq!(connected, ConnectionOutcome::Succeeded);
    assert!(is_connected(&manager, "local").await);

    let result = QueryService::new(manager.clone())
        .execute("local", "main", "SELECT 1 AS value")
        .await
        .expect("query");
    assert_eq!(result.rows, vec![vec![serde_json::json!(1)]]);

    assert!(manager.disconnect_local("local").await);
    assert!(!is_connected(&manager, "local").await);
    assert!(!manager.disconnect_local("local").await);
}

#[tokio::test]
async fn snapshot_disconnects_a_session_after_an_external_profile_change() {
    let (_directory, manager) = manager();
    let created = manager
        .repository
        .create(sqlite_config("local"), false)
        .await
        .expect("create profile");
    manager.connect("local").await.expect("connect");

    let mut updated = sqlite_config("local");
    updated.name = "Renamed".to_string();
    manager
        .repository
        .save(SaveConnectionRequest {
            config: updated,
            expected_revision: Some(created.revision),
            mcp_enabled: false,
            group_name: None,
            tags: Vec::new(),
        })
        .await
        .expect("update profile");

    let (snapshot, _) = manager
        .snapshot_with_session_generations()
        .await
        .expect("snapshot");
    assert_eq!(snapshot.profiles[0].name, "Renamed");
    assert!(!is_connected(&manager, "local").await);
}

#[tokio::test]
async fn test_connection_rejects_reusing_a_password_for_a_changed_endpoint() {
    let (_directory, manager) = manager();
    let mut saved = sqlite_config("local");
    saved.password = "not-a-real-password".to_string();
    manager
        .repository
        .create(saved, false)
        .await
        .expect("create profile");

    let mut candidate = sqlite_config("local");
    candidate.host = "different.sqlite3".to_string();
    let error = manager
        .test_connection(candidate)
        .await
        .expect_err("changed endpoint must require password re-entry");

    assert!(error.contains("不能复用旧密码"));
}
