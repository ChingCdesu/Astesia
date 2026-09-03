use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::{
    options::{ClientOptions, Credential, ServerAddress},
    Client as MongoClient,
};
use std::time::Instant;

use super::{
    bytes_to_hex, ColumnInfo, ConnectionConfig, DatabaseDriver, DbType, DocumentPage, IndexInfo,
    QueryResult, TableInfo, TableRef,
};

pub struct MongoDriver {
    config: ConnectionConfig,
    client: Option<MongoClient>,
}

impl MongoDriver {
    pub fn new(config: ConnectionConfig) -> Self {
        Self {
            config,
            client: None,
        }
    }

    /// Build typed client options so credentials with special characters
    /// (`/ # ? @ :`, spaces, …) are handled by the driver instead of being
    /// string-interpolated into a URI, which previously mis-parsed and produced
    /// errors like "invalid port number".
    fn client_options(&self) -> ClientOptions {
        let credential = if self.config.username.is_empty() {
            None
        } else {
            Some(
                Credential::builder()
                    .username(self.config.username.clone())
                    .password(self.config.password.clone())
                    .build(),
            )
        };
        ClientOptions::builder()
            .hosts(vec![ServerAddress::Tcp {
                host: self.config.host.clone(),
                port: Some(self.config.port),
            }])
            .credential(credential)
            .build()
    }

    fn client(&self) -> anyhow::Result<&MongoClient> {
        self.client
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))
    }

    fn bson_to_json(val: &mongodb::bson::Bson) -> serde_json::Value {
        use mongodb::bson::Bson;
        use serde_json::Value as J;
        match val {
            Bson::Double(v) => serde_json::Number::from_f64(*v)
                .map(J::Number)
                .unwrap_or(J::Null),
            Bson::String(v) => J::String(v.clone()),
            Bson::Boolean(v) => J::Bool(*v),
            Bson::Null | Bson::Undefined => J::Null,
            Bson::Int32(v) => J::Number((*v).into()),
            Bson::Int64(v) => J::Number((*v).into()),
            Bson::Decimal128(v) => J::String(v.to_string()),
            Bson::ObjectId(v) => J::String(v.to_hex()),
            // ISO 8601 (e.g. 2024-01-02T03:04:05.678Z); fall back to Debug-ish on overflow.
            Bson::DateTime(v) => {
                J::String(v.try_to_rfc3339_string().unwrap_or_else(|_| v.to_string()))
            }
            Bson::Timestamp(t) => J::String(format!("Timestamp({}, {})", t.time, t.increment)),
            Bson::Binary(b) => J::String(bytes_to_hex(&b.bytes, "0x")),
            Bson::RegularExpression(r) => J::String(format!("/{}/{}", r.pattern, r.options)),
            Bson::Symbol(s) => J::String(s.clone()),
            Bson::JavaScriptCode(c) => J::String(c.clone()),
            Bson::JavaScriptCodeWithScope(c) => J::String(c.code.clone()),
            Bson::MinKey => J::String("MinKey".to_string()),
            Bson::MaxKey => J::String("MaxKey".to_string()),
            Bson::Array(arr) => J::Array(arr.iter().map(Self::bson_to_json).collect()),
            Bson::Document(doc) => {
                let map: serde_json::Map<String, J> = doc
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::bson_to_json(v)))
                    .collect();
                J::Object(map)
            }
            other => J::String(format!("{:?}", other)),
        }
    }
}

#[async_trait]
impl DatabaseDriver for MongoDriver {
    async fn connect(&mut self) -> anyhow::Result<()> {
        let client = MongoClient::with_options(self.client_options())?;
        self.client = Some(client);
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        self.client = None;
        Ok(())
    }

    async fn test_connection(&self) -> anyhow::Result<bool> {
        let client = MongoClient::with_options(self.client_options())?;
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
        collections
            .into_iter()
            .map(|name| -> anyhow::Result<_> {
                Ok(TableInfo {
                    reference: TableRef::unqualified(name),
                    row_count: None,
                    comment: Some("collection".to_string()),
                })
            })
            .collect()
    }

    async fn get_columns(
        &self,
        database: &str,
        table: &TableRef,
    ) -> anyhow::Result<Vec<ColumnInfo>> {
        let client = self.client()?;
        let db = client.database(database);
        let collection = db.collection::<mongodb::bson::Document>(table.name());
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

    async fn get_indexes(
        &self,
        database: &str,
        table: &TableRef,
    ) -> anyhow::Result<Vec<IndexInfo>> {
        let client = self.client()?;
        let db = client.database(database);
        let collection = db.collection::<mongodb::bson::Document>(table.name());
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
                    let filter: mongodb::bson::Document =
                        if filter_str.is_empty() || filter_str == "{}" {
                            mongodb::bson::doc! {}
                        } else {
                            serde_json::from_str::<serde_json::Value>(filter_str)
                                .ok()
                                .and_then(|v| mongodb::bson::to_document(&v).ok())
                                .unwrap_or_default()
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
        table: &TableRef,
        page: u32,
        page_size: u32,
    ) -> anyhow::Result<QueryResult> {
        let client = self.client()?;
        let db = client.database(database);
        let collection = db.collection::<mongodb::bson::Document>(table.name());
        let start = Instant::now();

        let skip = ((page - 1) * page_size) as u64;
        let limit = page_size as i64;
        let options = mongodb::options::FindOptions::builder()
            .skip(skip)
            .limit(limit)
            .build();
        let mut cursor = collection
            .find(mongodb::bson::doc! {})
            .with_options(options)
            .await?;
        let mut docs = Vec::new();
        while let Some(doc) = cursor.try_next().await? {
            docs.push(doc);
        }
        let elapsed = start.elapsed().as_millis() as u64;
        self.docs_to_result(docs, elapsed)
    }

    async fn get_documents(
        &self,
        database: &str,
        collection: &TableRef,
        filter: Option<serde_json::Value>,
        page: u32,
        page_size: u32,
    ) -> anyhow::Result<DocumentPage> {
        anyhow::ensure!(page > 0, "MongoDB page numbers start at 1");
        anyhow::ensure!(page_size > 0, "MongoDB page size must be positive");
        let client = self.client()?;
        let collection = client
            .database(database)
            .collection::<mongodb::bson::Document>(collection.name());
        let filter = match filter {
            None => mongodb::bson::doc! {},
            Some(serde_json::Value::Object(fields)) => {
                mongodb::bson::to_document(&serde_json::Value::Object(fields))?
            }
            Some(_) => anyhow::bail!("MongoDB filter must be a JSON object"),
        };
        let total_documents = collection.count_documents(filter.clone()).await?;
        let skip = u64::from(page - 1)
            .checked_mul(u64::from(page_size))
            .ok_or_else(|| anyhow::anyhow!("MongoDB page offset is too large"))?;
        let options = mongodb::options::FindOptions::builder()
            .skip(skip)
            .limit(i64::from(page_size))
            .build();
        let mut cursor = collection.find(filter).with_options(options).await?;
        let mut documents = Vec::new();
        while let Some(document) = cursor.try_next().await? {
            documents.push(mongodb::bson::Bson::Document(document).into_relaxed_extjson());
        }
        Ok(DocumentPage {
            documents,
            total_documents,
        })
    }

    fn db_type(&self) -> DbType {
        DbType::MongoDB
    }
}

impl MongoDriver {
    fn docs_to_result(
        &self,
        docs: Vec<mongodb::bson::Document>,
        elapsed: u64,
    ) -> anyhow::Result<QueryResult> {
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
