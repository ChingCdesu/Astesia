use std::{collections::HashMap, error::Error, fmt};

use crate::db::{ColumnInfo, DbType, TableRef};

use super::{connections::ConnectionManager, QueryTarget};

#[derive(Clone, Debug)]
pub(crate) struct ErTable {
    pub(crate) reference: TableRef,
    pub(crate) columns: Vec<ColumnInfo>,
}

#[derive(Clone, Debug)]
pub(crate) struct ErRelationship {
    pub(crate) name: String,
    pub(crate) from_table: TableRef,
    pub(crate) from_columns: Vec<String>,
    pub(crate) to_table: TableRef,
    pub(crate) to_columns: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ErSchema {
    pub(crate) tables: Vec<ErTable>,
    pub(crate) relationships: Vec<ErRelationship>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ErLoadError {
    Connection(String),
    SessionChanged { expected: u64, actual: u64 },
    EngineChanged { expected: DbType, actual: DbType },
    Unsupported(DbType),
    Tables(String),
    Columns { table: TableRef, message: String },
    ForeignKeys { table: TableRef, message: String },
    BackgroundTask(String),
}

impl fmt::Display for ErLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connection(message) | Self::BackgroundTask(message) => {
                formatter.write_str(message)
            }
            Self::SessionChanged { expected, actual } => write!(
                formatter,
                "Connection session changed while the ER diagram loaded (expected {expected}, found {actual})"
            ),
            Self::EngineChanged { expected, actual } => write!(
                formatter,
                "Connection engine changed while the ER diagram loaded (expected {expected:?}, found {actual:?})"
            ),
            Self::Unsupported(db_type) => {
                write!(formatter, "ER diagrams are not supported for {db_type:?}")
            }
            Self::Tables(message) => write!(formatter, "Could not load tables: {message}"),
            Self::Columns { table, message } => {
                write!(formatter, "Could not load columns for {table}: {message}")
            }
            Self::ForeignKeys { table, message } => {
                write!(formatter, "Could not load foreign keys for {table}: {message}")
            }
        }
    }
}

impl Error for ErLoadError {}

#[derive(Clone)]
pub(crate) struct ErDiagramService {
    manager: ConnectionManager,
}

impl ErDiagramService {
    pub(super) fn new(manager: ConnectionManager) -> Self {
        Self { manager }
    }

    pub(crate) async fn load(&self, target: &QueryTarget) -> Result<ErSchema, ErLoadError> {
        let (handle, actual_generation) = self
            .manager
            .driver_session(&target.connection_id)
            .await
            .map_err(ErLoadError::Connection)?;
        if actual_generation != target.session_generation {
            return Err(ErLoadError::SessionChanged {
                expected: target.session_generation,
                actual: actual_generation,
            });
        }
        let driver = handle
            .lock_active()
            .await
            .map_err(ErLoadError::Connection)?;
        let actual_db_type = driver.db_type();
        if actual_db_type != target.db_type {
            return Err(ErLoadError::EngineChanged {
                expected: target.db_type,
                actual: actual_db_type,
            });
        }
        if !actual_db_type.capabilities().foreign_keys {
            return Err(ErLoadError::Unsupported(actual_db_type));
        }

        let tables = driver
            .get_tables(&target.database)
            .await
            .map_err(|error| ErLoadError::Tables(error.to_string()))?;
        let mut schema_tables = Vec::with_capacity(tables.len());
        let mut relationships = Vec::new();
        for table in tables {
            let reference = table.reference;
            let columns = driver
                .get_columns(&target.database, &reference)
                .await
                .map_err(|error| ErLoadError::Columns {
                    table: reference.clone(),
                    message: error.to_string(),
                })?;
            let foreign_keys = driver
                .get_foreign_keys(&target.database, &reference)
                .await
                .map_err(|error| ErLoadError::ForeignKeys {
                    table: reference.clone(),
                    message: error.to_string(),
                })?;
            relationships.extend(foreign_keys.into_iter().map(|foreign_key| ErRelationship {
                name: foreign_key.name,
                from_table: foreign_key.from_table,
                from_columns: foreign_key.from_columns,
                to_table: foreign_key.to_table,
                to_columns: foreign_key.to_columns,
            }));
            schema_tables.push(ErTable { reference, columns });
        }
        schema_tables.sort_by(|left, right| left.reference.cmp(&right.reference));
        relationships.sort_by(|left, right| {
            (&left.from_table, &left.to_table, &left.name).cmp(&(
                &right.from_table,
                &right.to_table,
                &right.name,
            ))
        });
        Ok(ErSchema {
            tables: schema_tables,
            relationships,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ErPoint {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct ErLayoutNode {
    pub(crate) table: usize,
    pub(crate) position: ErPoint,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ErLayout {
    pub(crate) nodes: Vec<ErLayoutNode>,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ErBounds {
    pub(crate) origin: ErPoint,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl ErLayout {
    pub(crate) const HEADER_HEIGHT: f32 = 34.0;
    pub(crate) const ROW_HEIGHT: f32 = 28.0;

    pub(crate) fn build(schema: &ErSchema) -> Self {
        const NODE_WIDTH: f32 = 260.0;
        const RANK_GAP: f32 = 140.0;
        const NODE_GAP: f32 = 48.0;
        const MARGIN: f32 = 48.0;

        if schema.tables.is_empty() {
            return Self::default();
        }
        let indexes = schema
            .tables
            .iter()
            .enumerate()
            .map(|(index, table)| (table.reference.clone(), index))
            .collect::<HashMap<_, _>>();
        let mut ranks = vec![0_usize; schema.tables.len()];
        for _ in 0..schema.tables.len() {
            let mut changed = false;
            for relationship in &schema.relationships {
                let (Some(from), Some(to)) = (
                    indexes.get(&relationship.from_table),
                    indexes.get(&relationship.to_table),
                ) else {
                    continue;
                };
                let next = ranks[*to].saturating_add(1).min(schema.tables.len() - 1);
                if next > ranks[*from] {
                    ranks[*from] = next;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        let rank_count = ranks.iter().copied().max().unwrap_or(0) + 1;
        let mut tables_by_rank = vec![Vec::<usize>::new(); rank_count];
        for (table, rank) in ranks.into_iter().enumerate() {
            tables_by_rank[rank].push(table);
        }
        let mut nodes = Vec::with_capacity(schema.tables.len());
        let mut width = MARGIN;
        let mut height = MARGIN;
        for (rank, tables) in tables_by_rank.iter().enumerate() {
            let x = MARGIN + rank as f32 * (NODE_WIDTH + RANK_GAP);
            let mut y = MARGIN;
            for table in tables {
                let node_height = Self::HEADER_HEIGHT
                    + schema.tables[*table].columns.len().min(12) as f32 * Self::ROW_HEIGHT
                    + if schema.tables[*table].columns.len() > 12 {
                        Self::ROW_HEIGHT
                    } else {
                        0.0
                    };
                nodes.push(ErLayoutNode {
                    table: *table,
                    position: ErPoint { x, y },
                    width: NODE_WIDTH,
                    height: node_height,
                });
                y += node_height + NODE_GAP;
                height = height.max(y);
            }
            width = width.max(x + NODE_WIDTH + MARGIN);
        }
        nodes.sort_by_key(|node| node.table);
        Self {
            nodes,
            width,
            height: height + MARGIN,
        }
    }

    pub(crate) fn bounds(&self, offsets: &[ErPoint]) -> Option<ErBounds> {
        let mut minimum_x = f32::INFINITY;
        let mut minimum_y = f32::INFINITY;
        let mut maximum_x = f32::NEG_INFINITY;
        let mut maximum_y = f32::NEG_INFINITY;
        for node in &self.nodes {
            let offset = offsets
                .get(node.table)
                .copied()
                .unwrap_or(ErPoint { x: 0.0, y: 0.0 });
            let x = node.position.x + offset.x;
            let y = node.position.y + offset.y;
            minimum_x = minimum_x.min(x);
            minimum_y = minimum_y.min(y);
            maximum_x = maximum_x.max(x + node.width);
            maximum_y = maximum_y.max(y + node.height);
        }
        self.nodes.first().map(|_| ErBounds {
            origin: ErPoint {
                x: minimum_x,
                y: minimum_y,
            },
            width: maximum_x - minimum_x,
            height: maximum_y - minimum_y,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ErLoadRequest {
    generation: u64,
}

#[derive(Debug)]
enum ErPhase {
    Idle,
    Loading {
        generation: u64,
        schema: Option<ErSchema>,
    },
    Ready(ErSchema),
    Failed {
        error: ErLoadError,
        schema: Option<ErSchema>,
    },
    Unavailable(String),
}

pub(crate) enum ErStatus<'a> {
    Idle,
    Loading(Option<&'a ErSchema>),
    Ready(&'a ErSchema),
    Failed(&'a ErLoadError, Option<&'a ErSchema>),
    Unavailable(&'a str),
}

pub(crate) struct ErDiagramState {
    target: QueryTarget,
    next_generation: u64,
    phase: ErPhase,
}

impl ErDiagramState {
    pub(crate) fn new(target: QueryTarget) -> Self {
        Self {
            target,
            next_generation: 0,
            phase: ErPhase::Idle,
        }
    }

    pub(crate) fn target(&self) -> &QueryTarget {
        &self.target
    }

    pub(crate) fn status(&self) -> ErStatus<'_> {
        match &self.phase {
            ErPhase::Idle => ErStatus::Idle,
            ErPhase::Loading { schema, .. } => ErStatus::Loading(schema.as_ref()),
            ErPhase::Ready(schema) => ErStatus::Ready(schema),
            ErPhase::Failed { error, schema } => ErStatus::Failed(error, schema.as_ref()),
            ErPhase::Unavailable(reason) => ErStatus::Unavailable(reason),
        }
    }

    pub(crate) fn begin_load(&mut self) -> Option<ErLoadRequest> {
        if matches!(
            self.phase,
            ErPhase::Loading { .. } | ErPhase::Unavailable(_)
        ) {
            return None;
        }
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("ER diagram request generation exhausted");
        let request = ErLoadRequest {
            generation: self.next_generation,
        };
        let schema = match std::mem::replace(&mut self.phase, ErPhase::Idle) {
            ErPhase::Ready(schema)
            | ErPhase::Failed {
                schema: Some(schema),
                ..
            } => Some(schema),
            _ => None,
        };
        self.phase = ErPhase::Loading {
            generation: request.generation,
            schema,
        };
        Some(request)
    }

    pub(crate) fn finish_load(
        &mut self,
        request: ErLoadRequest,
        result: Result<ErSchema, ErLoadError>,
    ) -> bool {
        if !matches!(self.phase, ErPhase::Loading { generation, .. } if generation == request.generation)
        {
            return false;
        }
        let previous = match std::mem::replace(&mut self.phase, ErPhase::Idle) {
            ErPhase::Loading { schema, .. } => schema,
            _ => None,
        };
        self.phase = match result {
            Ok(schema) => ErPhase::Ready(schema),
            Err(error) => ErPhase::Failed {
                error,
                schema: previous,
            },
        };
        true
    }

    pub(crate) fn invalidate_session(
        &mut self,
        connection_id: &str,
        session_generation: u64,
        reason: impl Into<String>,
    ) -> bool {
        if self.target.connection_id != connection_id
            || self.target.session_generation != session_generation
            || matches!(self.phase, ErPhase::Unavailable(_))
        {
            return false;
        }
        self.phase = ErPhase::Unavailable(reason.into());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(schema: Option<&str>, name: &str, column_count: usize) -> ErTable {
        ErTable {
            reference: TableRef::from_parts(schema.map(str::to_string), name.to_string()),
            columns: (0..column_count)
                .map(|index| ColumnInfo {
                    name: format!("column_{index}"),
                    data_type: "bigint".to_string(),
                    nullable: false,
                    is_primary_key: index == 0,
                    default_value: None,
                    comment: None,
                })
                .collect(),
        }
    }

    fn relationship(from: TableRef, to: TableRef) -> ErRelationship {
        ErRelationship {
            name: "fk".to_string(),
            from_table: from,
            from_columns: vec!["parent_id".to_string()],
            to_table: to,
            to_columns: vec!["id".to_string()],
        }
    }

    #[test]
    fn empty_small_and_large_schemas_have_finite_layouts() {
        assert!(ErLayout::build(&ErSchema {
            tables: vec![],
            relationships: vec![]
        })
        .nodes
        .is_empty());
        for count in [2, 80] {
            let schema = ErSchema {
                tables: (0..count)
                    .map(|index| table(Some("public"), &format!("table_{index}"), index % 16))
                    .collect(),
                relationships: vec![],
            };
            let layout = ErLayout::build(&schema);
            assert_eq!(layout.nodes.len(), count);
            assert!(layout.width.is_finite() && layout.height.is_finite());
            assert!(layout.width > 0.0 && layout.height > 0.0);
        }
    }

    #[test]
    fn qualified_table_identity_keeps_same_named_tables_distinct() {
        let first = table(Some("public"), "users", 1);
        let second = table(Some("audit"), "users", 1);
        let schema = ErSchema {
            tables: vec![first, second],
            relationships: vec![],
        };

        let layout = ErLayout::build(&schema);
        assert_eq!(layout.nodes.len(), 2);
        assert_ne!(schema.tables[0].reference, schema.tables[1].reference);
    }

    #[test]
    fn cycles_are_bounded_and_do_not_break_layout() {
        let parent = TableRef::qualified("public", "parents");
        let child = TableRef::qualified("public", "children");
        let schema = ErSchema {
            tables: vec![
                table(Some("public"), "parents", 2),
                table(Some("public"), "children", 2),
            ],
            relationships: vec![
                relationship(parent.clone(), child.clone()),
                relationship(child, parent),
            ],
        };

        let layout = ErLayout::build(&schema);
        assert_eq!(layout.nodes.len(), 2);
        assert!(layout.width < 2_000.0);
    }

    #[test]
    fn bounds_include_dragged_node_offsets() {
        let schema = ErSchema {
            tables: vec![
                table(Some("public"), "first", 1),
                table(Some("public"), "second", 1),
            ],
            relationships: vec![],
        };
        let layout = ErLayout::build(&schema);
        let original = layout.bounds(&[]).expect("layout bounds");
        let dragged = layout
            .bounds(&[ErPoint { x: -100.0, y: 0.0 }, ErPoint { x: 500.0, y: 0.0 }])
            .expect("dragged bounds");

        assert_eq!(dragged.origin.x, original.origin.x - 100.0);
        assert!(dragged.width > original.width + 500.0);
    }
}
