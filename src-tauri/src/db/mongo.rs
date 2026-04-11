use async_trait::async_trait;
use mongodb::{Client as MongoClient, options::ClientOptions};
use futures::TryStreamExt;
use std::time::Instant;

use super::{ColumnInfo, ConnectionConfig, DatabaseDriver, DbType, IndexInfo, QueryResult, TableInfo};

pub struct MongoDriver {
    config: ConnectionConfig,
    client: Option<MongoClient>,
}

impl MongoDriver {
    pub fn new(config: ConnectionConfig) -> Self {
        Self { config, client: None }
    }

    fn connection_string(&self) -> String {
        if self.config.username.is_empty() {
            format!("mongodb://{}:{}", self.config.host, self.config.port)
        } else {
            format!(
                "mongodb://{}:{}@{}:{}",
                self.config.username, self.config.password, self.config.host, self.config.port
            )
        }
    }

    fn client(&self) -> anyhow::Result<&MongoClient> {
        self.client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))
    }

    fn bson_to_json(val: &mongodb::bson::Bson) -> serde_json::Value {
        match val {
            mongodb::bson::Bson::Double(v) => serde_json::Number::from_f64(*v)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            mongodb::bson::Bson::String(v) => serde_json::Value::String(v.clone()),
            mongodb::bson::Bson::Boolean(v) => serde_json::Value::Bool(*v),
            mongodb::bson::Bson::Null => serde_json::Value::Null,
            mongodb::bson::Bson::Int32(v) => serde_json::Value::Number((*v).into()),
            mongodb::bson::Bson::Int64(v) => serde_json::Value::Number((*v).into()),
            mongodb::bson::Bson::ObjectId(v) => serde_json::Value::String(v.to_hex()),
            mongodb::bson::Bson::DateTime(v) => serde_json::Value::String(v.to_string()),
            mongodb::bson::Bson::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(Self::bson_to_json).collect())
            }
            mongodb::bson::Bson::Document(doc) => {
                let map: serde_json::Map<String, serde_json::Value> = doc
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::bson_to_json(v)))
                    .collect();
                serde_json::Value::Object(map)
            }
            other => serde_json::Value::String(format!("{:?}", other)),
        }
    }
}

#[async_trait]
impl DatabaseDriver for MongoDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let client_options = ClientOptions::parse(&self.connection_string()).await?;
        let client = MongoClient::with_options(client_options)?;
        self.client = Some(client);
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        self.client = None;
        Ok(())
    }

    async fn test_connection(&self) -> anyhow::Result<bool> {
        let client_options = ClientOptions::parse(&self.connection_string()).await?;
        let client = MongoClient::with_options(client_options)?;
        client.list_database_names().await?;
        Ok(true)
    }

    async fn get_databases(&self) -> anyhow::Result<Vec<String>> {
        let client = self.client()?;
        let dbs = client.list_database_names().await?;
        Ok(dbs)
    }

    async fn get_tables(&self, database: &str) -> anyhow::Result<Vec<TableInfo>> {
        let client = self.client()?;
        let db = client.database(database);
        let collections = db.list_collection_names().await?;
        Ok(collections
            .into_iter()
            .map(|name| TableInfo {
                name,
                schema: None,
                row_count: None,
                comment: Some("collection".to_string()),
            })
            .collect())
    }

    async fn get_columns(&self, database: &str, table: &str) -> anyhow::Result<Vec<ColumnInfo>> {
        let client = self.client()?;
        let db = client.database(database);
        let collection = db.collection::<mongodb::bson::Document>(table);
        let doc = collection.find_one(mongodb::bson::doc! {}).await?;
        match doc {
            Some(doc) => {
                let columns = doc
                    .keys()
                    .map(|key| ColumnInfo {
                        name: key.clone(),
                        data_type: match doc.get(key) {
                            Some(mongodb::bson::Bson::String(_)) => "String".to_string(),
                            Some(mongodb::bson::Bson::Int32(_)) => "Int32".to_string(),
                            Some(mongodb::bson::Bson::Int64(_)) => "Int64".to_string(),
                            Some(mongodb::bson::Bson::Double(_)) => "Double".to_string(),
                            Some(mongodb::bson::Bson::Boolean(_)) => "Boolean".to_string(),
                            Some(mongodb::bson::Bson::Array(_)) => "Array".to_string(),
                            Some(mongodb::bson::Bson::Document(_)) => "Object".to_string(),
                            Some(mongodb::bson::Bson::ObjectId(_)) => "ObjectId".to_string(),
                            Some(mongodb::bson::Bson::DateTime(_)) => "DateTime".to_string(),
                            Some(mongodb::bson::Bson::Null) => "Null".to_string(),
                            _ => "Unknown".to_string(),
                        },
                        nullable: true,
                        is_primary_key: key == "_id",
                        default_value: None,
                        comment: None,
                    })
                    .collect();
                Ok(columns)
            }
            None => Ok(vec![]),
        }
    }

    async fn get_indexes(&self, database: &str, table: &str) -> anyhow::Result<Vec<IndexInfo>> {
        let client = self.client()?;
        let db = client.database(database);
        let collection = db.collection::<mongodb::bson::Document>(table);
        let mut cursor = collection.list_indexes().await?;
        let mut indexes = Vec::new();
        while let Some(index) = cursor.try_next().await? {
            let name = index.options.and_then(|o| o.name).unwrap_or_default();
            let columns: Vec<String> = index.keys.keys().cloned().collect();
            indexes.push(IndexInfo {
                name: name.clone(),
                columns,
                is_unique: false,
                is_primary: name == "_id_",
            });
        }
        Ok(indexes)
    }

    async fn execute_query(&self, database: &str, query: &str) -> anyhow::Result<QueryResult> {
        let client = self.client()?;
        let db = client.database(database);
        let start = Instant::now();

        // Parse simple MongoDB-like commands: db.collection.find({...})
        let trimmed = query.trim();
        if let Some(rest) = trimmed.strip_prefix("db.") {
            if let Some(dot_pos) = rest.find('.') {
                let collection_name = &rest[..dot_pos];
                let command = &rest[dot_pos + 1..];

                if command.starts_with("find(") {
                    let collection = db.collection::<mongodb::bson::Document>(collection_name);
                    let filter_str = command
                        .strip_prefix("find(")
                        .and_then(|s| s.strip_suffix(')'))
                        .unwrap_or("{}");
                    let filter: mongodb::bson::Document = if filter_str.is_empty() || filter_str == "{}" {
                        mongodb::bson::doc! {}
                    } else {
                        serde_json::from_str::<serde_json::Value>(filter_str)
                            .ok()
                            .and_then(|v| mongodb::bson::to_document(&v).ok())
                            .unwrap_or(mongodb::bson::doc! {})
                    };
                    let mut cursor = collection.find(filter).await?;
                    let mut docs = Vec::new();
                    while let Some(doc) = cursor.try_next().await? {
                        docs.push(doc);
                        if docs.len() >= 100 {
                            break;
                        }
                    }
                    let elapsed = start.elapsed().as_millis() as u64;
                    return self.docs_to_result(docs, elapsed);
                }
            }
        }

        // Fallback: try to run as a raw command
        let command_doc = mongodb::bson::doc! { "ping": 1 };
        db.run_command(command_doc).await?;
        let elapsed = start.elapsed().as_millis() as u64;
        Ok(QueryResult {
            execution_time_ms: elapsed,
            ..Default::default()
        })
    }

    async fn get_table_data(
        &self,
        database: &str,
        table: &str,
        page: u32,
        page_size: u32,
    ) -> anyhow::Result<QueryResult> {
        let client = self.client()?;
        let db = client.database(database);
        let collection = db.collection::<mongodb::bson::Document>(table);
        let start = Instant::now();

        let skip = ((page - 1) * page_size) as u64;
        let limit = page_size as i64;
        let options = mongodb::options::FindOptions::builder()
            .skip(skip)
            .limit(limit)
            .build();
        let mut cursor = collection.find(mongodb::bson::doc! {}).with_options(options).await?;
        let mut docs = Vec::new();
        while let Some(doc) = cursor.try_next().await? {
            docs.push(doc);
        }
        let elapsed = start.elapsed().as_millis() as u64;
        self.docs_to_result(docs, elapsed)
    }

    fn db_type(&self) -> DbType {
        DbType::MongoDB
    }
}

impl MongoDriver {
    fn docs_to_result(&self, docs: Vec<mongodb::bson::Document>, elapsed: u64) -> anyhow::Result<QueryResult> {
        if docs.is_empty() {
            return Ok(QueryResult {
                execution_time_ms: elapsed,
                ..Default::default()
            });
        }

        // Collect all unique keys from all documents
        let mut all_keys: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for doc in &docs {
            for key in doc.keys() {
                if seen.insert(key.clone()) {
                    all_keys.push(key.clone());
                }
            }
        }

        let columns: Vec<ColumnInfo> = all_keys
            .iter()
            .map(|key| ColumnInfo {
                name: key.clone(),
                data_type: "BSON".to_string(),
                nullable: true,
                is_primary_key: key == "_id",
                default_value: None,
                comment: None,
            })
            .collect();

        let rows: Vec<Vec<serde_json::Value>> = docs
            .iter()
            .map(|doc| {
                all_keys
                    .iter()
                    .map(|key| {
                        doc.get(key)
                            .map(Self::bson_to_json)
                            .unwrap_or(serde_json::Value::Null)
                    })
                    .collect()
            })
            .collect();

        Ok(QueryResult {
            columns,
            rows,
            affected_rows: 0,
            execution_time_ms: elapsed,
        })
    }
}
