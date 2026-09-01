use std::collections::{HashMap, HashSet};

use crate::connection_runtime::DriverHandle;
use crate::db::{DbType, ForeignKeyInfo, IndexInfo, TableRef, UnsupportedFeature};

use super::{BackupContent, BackupOptions, DropTableMode};

pub(super) struct BackupPlan {
    pub(super) database: String,
    pub(super) db_type: DbType,
    pub(super) content: BackupContent,
    pub(super) drop_tables: DropTableMode,
    pub(super) output_path: String,
    pub(super) tables: Vec<BackupTable>,
}

impl BackupPlan {
    pub(super) async fn discover(
        driver: &DriverHandle,
        database: String,
        options: BackupOptions,
    ) -> Result<Self, String> {
        let db_type = {
            let driver = driver.lock_active().await?;
            driver.db_type()
        };
        let capabilities = db_type.capabilities();
        if !capabilities.backup {
            return Err(UnsupportedFeature::new(db_type, "backup").to_string());
        }

        let raw_tables = match options.tables.as_ref() {
            Some(tables) => tables.clone(),
            None => discover_tables(driver, &database).await?,
        };
        let foreign_keys =
            discover_foreign_keys(driver, &database, &raw_tables, capabilities.foreign_keys)
                .await?;
        let mut indexes = discover_indexes(driver, &database, &raw_tables, db_type).await?;
        let tables = sort_tables_by_deps(&raw_tables, &foreign_keys)
            .into_iter()
            .map(|reference| {
                let table_indexes = indexes.remove(&reference).unwrap_or_default();
                BackupTable {
                    reference,
                    indexes: table_indexes,
                }
            })
            .collect();

        Ok(Self {
            database,
            db_type,
            content: options.content,
            drop_tables: options.drop_tables,
            output_path: options.output_path,
            tables,
        })
    }
}

pub(super) struct BackupTable {
    pub(super) reference: TableRef,
    pub(super) indexes: Vec<IndexInfo>,
}

async fn discover_tables(driver: &DriverHandle, database: &str) -> Result<Vec<TableRef>, String> {
    let driver = driver.lock_active().await?;
    let tables = driver
        .get_tables(database)
        .await
        .map_err(|error| error.to_string())?;
    Ok(tables.into_iter().map(|table| table.reference).collect())
}

async fn discover_foreign_keys(
    driver: &DriverHandle,
    database: &str,
    tables: &[TableRef],
    supported: bool,
) -> Result<HashMap<TableRef, Vec<ForeignKeyInfo>>, String> {
    if !supported {
        return Ok(HashMap::new());
    }

    let mut foreign_keys = HashMap::new();
    for table in tables {
        let keys = {
            let driver = driver.lock_active().await?;
            driver
                .get_foreign_keys(database, table)
                .await
                .map_err(|error| format!("读取表 {table} 的外键失败，无法生成可靠备份: {error}"))?
        };
        if !keys.is_empty() {
            foreign_keys.insert(table.clone(), keys);
        }
    }
    Ok(foreign_keys)
}

async fn discover_indexes(
    driver: &DriverHandle,
    database: &str,
    tables: &[TableRef],
    db_type: DbType,
) -> Result<HashMap<TableRef, Vec<IndexInfo>>, String> {
    if db_type != DbType::PostgreSQL {
        return Ok(HashMap::new());
    }

    let mut indexes = HashMap::new();
    for table in tables {
        let table_indexes = {
            let driver = driver.lock_active().await?;
            driver
                .get_indexes(database, table)
                .await
                .map_err(|error| format!("读取表 {table} 的索引失败，无法生成可靠备份: {error}"))?
        };
        if !table_indexes.is_empty() {
            indexes.insert(table.clone(), table_indexes);
        }
    }
    Ok(indexes)
}

fn sort_tables_by_deps(
    tables: &[TableRef],
    foreign_keys: &HashMap<TableRef, Vec<ForeignKeyInfo>>,
) -> Vec<TableRef> {
    let table_set: HashSet<&TableRef> = tables.iter().collect();
    let mut dependencies: HashMap<&TableRef, HashSet<&TableRef>> =
        tables.iter().map(|table| (table, HashSet::new())).collect();

    for (table, keys) in foreign_keys {
        let Some(table_dependencies) = dependencies.get_mut(table) else {
            continue;
        };
        for key in keys {
            if key.to_table != *table && table_set.contains(&key.to_table) {
                table_dependencies.insert(&key.to_table);
            }
        }
    }

    let mut in_degree: HashMap<&TableRef, usize> = tables
        .iter()
        .map(|table| (table, dependencies.get(table).map_or(0, HashSet::len)))
        .collect();
    let mut dependents: HashMap<&TableRef, Vec<&TableRef>> = HashMap::new();
    for (table, table_dependencies) in &dependencies {
        for dependency in table_dependencies {
            dependents.entry(*dependency).or_default().push(table);
        }
    }

    let mut ready: Vec<&TableRef> = in_degree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(table, _)| *table)
        .collect();
    ready.sort();
    let mut sorted = Vec::with_capacity(tables.len());
    while let Some(table) = ready.pop() {
        sorted.push(table.clone());
        if let Some(table_dependents) = dependents.get(table) {
            for dependent in table_dependents {
                if let Some(degree) = in_degree.get_mut(dependent) {
                    *degree = degree.saturating_sub(1);
                    if *degree == 0 {
                        ready.push(dependent);
                    }
                }
            }
        }
        ready.sort();
    }

    let sorted_set: HashSet<TableRef> = sorted.iter().cloned().collect();
    sorted.extend(
        tables
            .iter()
            .filter(|table| !sorted_set.contains(*table))
            .cloned(),
    );
    sorted
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::db::{ForeignKeyInfo, TableRef};

    use super::sort_tables_by_deps;

    fn table(schema: Option<&str>, name: &str) -> TableRef {
        TableRef::from_parts(schema.map(str::to_string), name.to_string())
    }

    fn foreign_key(from_table: &TableRef, to_table: &TableRef) -> ForeignKeyInfo {
        ForeignKeyInfo {
            name: format!("fk_{from_table}_{to_table}"),
            from_table: from_table.clone(),
            from_columns: vec!["parent_id".to_string()],
            to_table: to_table.clone(),
            to_columns: vec!["id".to_string()],
        }
    }

    #[test]
    fn orders_referenced_tables_before_dependents() {
        let comments = table(None, "comments");
        let posts = table(None, "posts");
        let users = table(None, "users");
        let tables = vec![comments.clone(), posts.clone(), users.clone()];
        let foreign_keys = HashMap::from([
            (comments.clone(), vec![foreign_key(&comments, &posts)]),
            (posts.clone(), vec![foreign_key(&posts, &users)]),
        ]);

        assert_eq!(
            sort_tables_by_deps(&tables, &foreign_keys),
            [users, posts, comments]
        );
    }

    #[test]
    fn preserves_input_order_for_dependency_cycles() {
        let left = table(None, "left");
        let right = table(None, "right");
        let tables = vec![left.clone(), right.clone()];
        let foreign_keys = HashMap::from([
            (left.clone(), vec![foreign_key(&left, &right)]),
            (right.clone(), vec![foreign_key(&right, &left)]),
        ]);

        assert_eq!(sort_tables_by_deps(&tables, &foreign_keys), tables);
    }

    #[test]
    fn dotted_schema_and_table_names_remain_distinct_during_dependency_ordering() {
        let parent = table(Some("billing.v2"), "account.history");
        let child = table(Some("billing.v2"), "entry.log");
        let tables = vec![child.clone(), parent.clone()];
        let foreign_keys = HashMap::from([(child.clone(), vec![foreign_key(&child, &parent)])]);

        assert_eq!(sort_tables_by_deps(&tables, &foreign_keys), [parent, child]);
    }
}
