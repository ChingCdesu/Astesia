use serde::{de::DeserializeOwned, Serialize};
use sqlx::{sqlite::SqliteConnectOptions, Row, SqlitePool};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::OnceCell;

const CACHE_FORMAT_VERSION: i32 = 1;

#[derive(Clone)]
pub(super) struct SchemaCache {
    path: PathBuf,
    pool: Arc<OnceCell<SqlitePool>>,
}

pub(super) struct CacheTicket {
    scope: String,
    epoch: i64,
    part: String,
}

impl SchemaCache {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            pool: Arc::default(),
        }
    }

    async fn pool(&self) -> Result<&SqlitePool, String> {
        self.pool
            .get_or_try_init(|| async {
                let options = SqliteConnectOptions::new()
                    .filename(&self.path)
                    .create_if_missing(true)
                    .busy_timeout(std::time::Duration::from_secs(5));
                let pool = SqlitePool::connect_with(options)
                    .await
                    .map_err(|error| error.to_string())?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS schema_scopes (
                        scope TEXT PRIMARY KEY,
                        connection_id TEXT NOT NULL,
                        database_name TEXT NOT NULL,
                        epoch INTEGER NOT NULL DEFAULT 0
                    )",
                )
                .execute(&pool)
                .await
                .map_err(|error| error.to_string())?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS schema_entries (
                        scope TEXT NOT NULL,
                        part TEXT NOT NULL,
                        epoch INTEGER NOT NULL,
                        payload TEXT NOT NULL,
                        PRIMARY KEY(scope, part)
                    )",
                )
                .execute(&pool)
                .await
                .map_err(|error| error.to_string())?;
                Ok(pool)
            })
            .await
    }

    pub(super) async fn read<T: DeserializeOwned>(
        &self,
        connection: &str,
        revision: i64,
        database: &str,
        part: &str,
    ) -> Result<(CacheTicket, Option<T>), String> {
        let pool = self.pool().await?;
        let scope = serde_json::to_string(&(CACHE_FORMAT_VERSION, connection, revision, database))
            .map_err(|error| error.to_string())?;
        sqlx::query(
            "INSERT OR IGNORE INTO schema_scopes(scope, connection_id, database_name)
             VALUES (?, ?, ?)",
        )
        .bind(&scope)
        .bind(connection)
        .bind(database)
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
        let row = sqlx::query(
            "SELECT s.epoch, e.payload
             FROM schema_scopes s
             LEFT JOIN schema_entries e
               ON e.scope = s.scope AND e.epoch = s.epoch AND e.part = ?
             WHERE s.scope = ?",
        )
        .bind(part)
        .bind(&scope)
        .fetch_one(pool)
        .await
        .map_err(|error| error.to_string())?;
        let epoch = row.get("epoch");
        let payload: Option<String> = row.get("payload");
        let value = payload.and_then(|json| serde_json::from_str(&json).ok());
        Ok((
            CacheTicket {
                scope,
                epoch,
                part: part.to_owned(),
            },
            value,
        ))
    }

    pub(super) async fn write<T: Serialize>(
        &self,
        ticket: CacheTicket,
        value: &T,
    ) -> Result<(), String> {
        let payload = serde_json::to_string(value).map_err(|error| error.to_string())?;
        // The generation check prevents an introspection started before Refresh from restoring stale data.
        sqlx::query(
            "INSERT INTO schema_entries(scope, part, epoch, payload)
             SELECT scope, ?, epoch, ? FROM schema_scopes WHERE scope = ? AND epoch = ?
             ON CONFLICT(scope, part)
             DO UPDATE SET epoch = excluded.epoch, payload = excluded.payload",
        )
        .bind(ticket.part)
        .bind(payload)
        .bind(ticket.scope)
        .bind(ticket.epoch)
        .execute(self.pool().await?)
        .await
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(super) async fn invalidate(
        &self,
        connection: &str,
        database: Option<&str>,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE schema_scopes SET epoch = epoch + 1
             WHERE connection_id = ? AND (? IS NULL OR database_name = ?)",
        )
        .bind(connection)
        .bind(database)
        .bind(database)
        .execute(self.pool().await?)
        .await
        .map_err(|error| error.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn persists_until_refresh_and_rejects_old_writes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("schema.sqlite3");
        let cache = SchemaCache::new(path.clone());
        let (ticket, value) = cache
            .read::<Vec<String>>("a", 1, "db", "tables")
            .await
            .unwrap();
        assert!(value.is_none());
        cache.write(ticket, &vec!["users"]).await.unwrap();
        let reopened = SchemaCache::new(path);
        assert_eq!(
            reopened
                .read::<Vec<String>>("a", 1, "db", "tables")
                .await
                .unwrap()
                .1
                .unwrap(),
            vec!["users"]
        );
        for (id, revision, db) in [("b", 1, "db"), ("a", 2, "db"), ("a", 1, "other")] {
            assert!(reopened
                .read::<Vec<String>>(id, revision, db, "tables")
                .await
                .unwrap()
                .1
                .is_none());
        }
        let (old, _) = cache
            .read::<Vec<String>>("a", 1, "db", "tables")
            .await
            .unwrap();
        reopened.invalidate("a", Some("db")).await.unwrap();
        cache.write(old, &vec!["stale"]).await.unwrap();
        let (fresh, value) = cache
            .read::<Vec<String>>("a", 1, "db", "tables")
            .await
            .unwrap();
        assert!(value.is_none());
        cache.write(fresh, &Vec::<String>::new()).await.unwrap();
        assert_eq!(
            reopened
                .read::<Vec<String>>("a", 1, "db", "tables")
                .await
                .unwrap()
                .1,
            Some(vec![])
        );
    }
}
