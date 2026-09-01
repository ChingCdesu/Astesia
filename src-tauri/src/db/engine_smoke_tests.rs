use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

use super::{create_driver, ConnectionConfig, DbType};

#[derive(Deserialize)]
struct SmokeTarget {
    config: ConnectionConfig,
    browse_database: Option<String>,
}

#[tokio::test]
#[ignore = "requires ASTESIA_ENGINE_SMOKE_CONFIG_PATH and seven disposable database engines"]
async fn all_engines_connect_browse_and_disconnect() {
    let path = std::env::var_os("ASTESIA_ENGINE_SMOKE_CONFIG_PATH")
        .map(PathBuf::from)
        .expect("ASTESIA_ENGINE_SMOKE_CONFIG_PATH is required");
    let targets: Vec<SmokeTarget> =
        serde_json::from_slice(&std::fs::read(path).expect("read engine smoke configuration"))
            .expect("parse engine smoke configuration");
    let engines = targets
        .iter()
        .map(|target| target.config.db_type)
        .collect::<HashSet<_>>();
    assert_eq!(engines, HashSet::from(DbType::all()));

    for target in targets {
        let engine = target.config.db_type;
        let mut driver = create_driver(&target.config);
        let tested = tokio::time::timeout(Duration::from_secs(45), driver.test_connection())
            .await
            .unwrap_or_else(|_| panic!("{engine:?} test timed out"))
            .unwrap_or_else(|error| panic!("{engine:?} test failed: {error}"));
        assert!(tested, "{engine:?} rejected the smoke configuration");
        tokio::time::timeout(Duration::from_secs(45), driver.connect())
            .await
            .unwrap_or_else(|_| panic!("{engine:?} connect timed out"))
            .unwrap_or_else(|error| panic!("{engine:?} connect failed: {error}"));

        let browse = async {
            let databases = driver.get_databases().await?;
            let database = target
                .browse_database
                .or_else(|| target.config.database.clone())
                .or_else(|| databases.first().cloned())
                .ok_or_else(|| anyhow::anyhow!("{engine:?} returned no browseable database"))?;
            driver.get_tables(&database).await?;
            anyhow::Ok(())
        }
        .await;
        let disconnect = driver.disconnect().await;

        browse.unwrap_or_else(|error| panic!("{engine:?} browse failed: {error}"));
        disconnect.unwrap_or_else(|error| panic!("{engine:?} disconnect failed: {error}"));
    }
}
