use std::collections::BTreeSet;

use serde_json::Value;

use crate::db::StatementResult;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResultCell {
    row: usize,
    column: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum Selection {
    #[default]
    None,
    Cells {
        anchor: ResultCell,
        extent: ResultCell,
    },
    Rows {
        anchor: usize,
        selected: BTreeSet<usize>,
    },
}

#[derive(Default)]
pub(super) struct QueryResultSelection {
    selection: Selection,
}

impl QueryResultSelection {
    pub(super) fn clear(&mut self) -> bool {
        self.replace(Selection::None)
    }

    pub(super) fn has_selection(&self) -> bool {
        !matches!(self.selection, Selection::None)
    }

    pub(super) fn select_cell(&mut self, row: usize, column: usize, extend: bool) -> bool {
        let cell = ResultCell { row, column };
        let anchor = match (&self.selection, extend) {
            (Selection::Cells { anchor, .. }, true) => *anchor,
            _ => cell,
        };
        self.replace(Selection::Cells {
            anchor,
            extent: cell,
        })
    }

    pub(super) fn select_row(&mut self, row: usize, extend: bool, toggle: bool) -> bool {
        let next = match (&self.selection, extend, toggle) {
            (Selection::Rows { anchor, selected }, true, _) => {
                let mut selected = selected.clone();
                selected.extend((*anchor).min(row)..=(*anchor).max(row));
                Selection::Rows {
                    anchor: *anchor,
                    selected,
                }
            }
            (Selection::Rows { selected, .. }, false, true) => {
                let mut selected = selected.clone();
                if !selected.insert(row) {
                    selected.remove(&row);
                }
                if selected.is_empty() {
                    Selection::None
                } else {
                    Selection::Rows {
                        anchor: row,
                        selected,
                    }
                }
            }
            _ => Selection::Rows {
                anchor: row,
                selected: BTreeSet::from([row]),
            },
        };
        self.replace(next)
    }

    pub(super) fn select_all_rows(&mut self, row_count: usize) -> bool {
        let next = if row_count == 0 {
            Selection::None
        } else {
            Selection::Rows {
                anchor: 0,
                selected: (0..row_count).collect(),
            }
        };
        self.replace(next)
    }

    pub(super) fn contains_cell(&self, row: usize, column: usize) -> bool {
        let Selection::Cells { anchor, extent } = self.selection else {
            return false;
        };
        (anchor.row.min(extent.row)..=anchor.row.max(extent.row)).contains(&row)
            && (anchor.column.min(extent.column)..=anchor.column.max(extent.column))
                .contains(&column)
    }

    pub(super) fn contains_row(&self, row: usize) -> bool {
        matches!(
            &self.selection,
            Selection::Rows { selected, .. } if selected.contains(&row)
        )
    }

    pub(super) fn to_tsv(&self, result: &StatementResult, include_headers: bool) -> Option<String> {
        let rows = match &self.selection {
            Selection::None => return None,
            Selection::Cells { anchor, extent } => {
                let row_start = anchor.row.min(extent.row);
                let row_end = anchor.row.max(extent.row);
                let column_start = anchor.column.min(extent.column);
                let column_end = anchor.column.max(extent.column);
                if row_end >= result.rows.len() || column_end >= result.columns.len() {
                    return None;
                }

                let mut rows = Vec::new();
                if include_headers {
                    rows.push(
                        result.columns[column_start..=column_end]
                            .iter()
                            .map(|column| column.name.clone())
                            .collect(),
                    );
                }
                for row in &result.rows[row_start..=row_end] {
                    let values = row.get(column_start..=column_end)?;
                    rows.push(values.iter().map(format_value).collect());
                }
                rows
            }
            Selection::Rows { selected, .. } => {
                let mut rows = Vec::new();
                if include_headers {
                    rows.push(
                        result
                            .columns
                            .iter()
                            .map(|column| column.name.clone())
                            .collect(),
                    );
                }
                for row_index in selected {
                    let row = result.rows.get(*row_index)?;
                    if row.len() < result.columns.len() {
                        return None;
                    }
                    rows.push(
                        row[..result.columns.len()]
                            .iter()
                            .map(format_value)
                            .collect(),
                    );
                }
                rows
            }
        };
        Some(format_tsv(rows))
    }

    fn replace(&mut self, next: Selection) -> bool {
        if self.selection == next {
            return false;
        }
        self.selection = next;
        true
    }
}

fn format_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) | Value::Bool(_) | Value::Number(_) => value.to_string(),
    }
}

fn format_tsv(rows: Vec<Vec<String>>) -> String {
    rows.into_iter()
        .map(|row| {
            row.into_iter()
                .map(escape_tsv_field)
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn escape_tsv_field(value: String) -> String {
    if value.contains(['\t', '\n', '\r', '"']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::db::{ColumnInfo, StatementResult};

    use super::*;

    fn result() -> StatementResult {
        StatementResult {
            sql: "SELECT * FROM events".to_string(),
            success: true,
            error: None,
            columns: ["id", "message", "metadata"]
                .into_iter()
                .map(|name| ColumnInfo {
                    name: name.to_string(),
                    data_type: "text".to_string(),
                    nullable: true,
                    is_primary_key: false,
                    default_value: None,
                    comment: None,
                })
                .collect(),
            rows: vec![
                vec![json!(1), json!("one\ttwo"), json!({ "x": 1 })],
                vec![json!(2), json!("line\nbreak"), Value::Null],
                vec![json!(3), json!("plain"), json!([1, 2])],
            ],
            affected_rows: 0,
            execution_time_ms: 1,
        }
    }

    #[test]
    fn rectangular_selection_copies_stable_quoted_tsv() {
        let result = result();
        let mut selection = QueryResultSelection::default();
        selection.select_cell(1, 2, false);
        selection.select_cell(0, 1, true);

        assert!(selection.contains_cell(0, 1));
        assert!(selection.contains_cell(1, 2));
        assert!(!selection.contains_cell(2, 1));
        assert_eq!(
            selection.to_tsv(&result, true).unwrap(),
            "message\tmetadata\n\"one\ttwo\"\t\"{\"\"x\"\":1}\"\n\"line\nbreak\"\t"
        );
    }

    #[test]
    fn row_selection_supports_replace_toggle_extend_and_headers() {
        let result = result();
        let mut selection = QueryResultSelection::default();
        selection.select_row(1, false, false);
        selection.select_row(0, false, true);
        selection.select_row(2, true, false);

        assert!(selection.contains_row(0));
        assert!(selection.contains_row(1));
        assert!(selection.contains_row(2));
        assert_eq!(
            selection.to_tsv(&result, true).unwrap(),
            "id\tmessage\tmetadata\n1\t\"one\ttwo\"\t\"{\"\"x\"\":1}\"\n2\t\"line\nbreak\"\t\n3\tplain\t[1,2]"
        );
    }

    #[test]
    fn select_all_and_clear_handle_empty_and_populated_results() {
        let result = result();
        let mut selection = QueryResultSelection::default();

        assert!(!selection.select_all_rows(0));
        assert!(selection.select_all_rows(result.rows.len()));
        assert!(selection.has_selection());
        assert!(selection.clear());
        assert!(!selection.has_selection());
        assert!(selection.to_tsv(&result, false).is_none());
    }
}
