use std::collections::HashMap;

use serde_json::Value;

use crate::db::ColumnInfo;

use super::{GridLoadError, GridLoadRequest, GridQuery, GridService, QueryTarget};
use crate::db::TableRef;

const CHART_PAGE_SIZE: u32 = 1_000;

#[derive(Clone)]
pub(crate) struct ChartService {
    grids: GridService,
}

#[derive(Clone, Debug)]
pub(crate) struct ChartTableData {
    pub(crate) columns: Vec<String>,
    pub(crate) rows: Vec<Vec<Value>>,
}

impl ChartService {
    pub(super) fn new(grids: GridService) -> Self {
        Self { grids }
    }

    pub(crate) async fn table_data(
        &self,
        target: QueryTarget,
        table: TableRef,
        query: GridQuery,
    ) -> Result<ChartTableData, GridLoadError> {
        let mut page_number = 1_u32;
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        loop {
            let request = GridLoadRequest::for_chart_page(
                target.clone(),
                table.clone(),
                query.clone(),
                page_number,
                CHART_PAGE_SIZE,
            );
            let page = self.grids.load(&request).await?;
            if columns.is_empty() {
                columns = page
                    .columns
                    .iter()
                    .map(|column| column.name.clone())
                    .collect();
            }
            let row_count = page.rows.len();
            rows.extend(page.rows);
            if row_count < CHART_PAGE_SIZE as usize {
                break;
            }
            page_number = page_number.saturating_add(1);
        }
        Ok(ChartTableData { columns, rows })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChartType {
    Bar,
    Line,
    Area,
    Scatter,
    Pie,
}

impl ChartType {
    pub(crate) const ALL: [Self; 5] = [Self::Bar, Self::Line, Self::Area, Self::Scatter, Self::Pie];
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChartPoint {
    pub(crate) label: String,
    pub(crate) x: f64,
    pub(crate) y: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChartSeries {
    pub(crate) name: String,
    pub(crate) points: Vec<ChartPoint>,
}

struct CategoricalAxis {
    labels: Vec<String>,
    indexes: HashMap<String, usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ChartDataError {
    Empty,
    NoNumericColumns,
    NoSeriesSelected,
    ScatterRequiresNumericX,
}

#[derive(Clone, Debug)]
pub(crate) struct ChartModel {
    columns: Vec<String>,
    numeric_columns: Vec<bool>,
    rows: Vec<Vec<Value>>,
    chart_type: ChartType,
    x_column: usize,
    y_columns: Vec<usize>,
}

impl ChartModel {
    pub(crate) fn new(columns: &[ColumnInfo], rows: &[Vec<Value>]) -> Self {
        Self::from_names(
            columns.iter().map(|column| column.name.clone()).collect(),
            rows,
        )
    }

    pub(crate) fn from_names(columns: Vec<String>, rows: &[Vec<Value>]) -> Self {
        let numeric_columns = (0..columns.len())
            .map(|column| column_is_numeric(rows, column))
            .collect::<Vec<_>>();
        let x_column = numeric_columns
            .iter()
            .position(|numeric| !numeric)
            .unwrap_or(0);
        let y_columns = numeric_columns
            .iter()
            .enumerate()
            .filter_map(|(index, numeric)| (*numeric && index != x_column).then_some(index))
            .take(3)
            .collect();
        Self {
            columns,
            numeric_columns,
            rows: rows.to_vec(),
            chart_type: ChartType::Bar,
            x_column,
            y_columns,
        }
    }

    pub(crate) fn replace_data(&mut self, columns: Vec<String>, rows: &[Vec<Value>]) {
        let chart_type = self.chart_type;
        let x_name = self.columns.get(self.x_column).cloned();
        let y_names = self
            .y_columns
            .iter()
            .filter_map(|column| self.columns.get(*column).cloned())
            .collect::<Vec<_>>();
        let mut replacement = Self::from_names(columns, rows);
        replacement.chart_type = chart_type;
        if let Some(x_column) = x_name
            .as_ref()
            .and_then(|name| replacement.columns.iter().position(|column| column == name))
        {
            replacement.x_column = x_column;
        }
        let y_columns = y_names
            .iter()
            .filter_map(|name| replacement.columns.iter().position(|column| column == name))
            .filter(|column| {
                replacement.numeric_columns[*column] && *column != replacement.x_column
            })
            .collect::<Vec<_>>();
        if !y_columns.is_empty() {
            replacement.y_columns = y_columns;
        }
        *self = replacement;
    }

    pub(crate) fn chart_type(&self) -> ChartType {
        self.chart_type
    }

    pub(crate) fn columns(&self) -> &[String] {
        &self.columns
    }

    pub(crate) fn numeric_columns(&self) -> &[bool] {
        &self.numeric_columns
    }

    pub(crate) fn x_column(&self) -> usize {
        self.x_column
    }

    pub(crate) fn y_columns(&self) -> &[usize] {
        &self.y_columns
    }

    pub(crate) fn set_chart_type(&mut self, chart_type: ChartType) -> bool {
        if self.chart_type == chart_type {
            return false;
        }
        self.chart_type = chart_type;
        true
    }

    pub(crate) fn set_x_column(&mut self, column: usize) -> bool {
        if column >= self.columns.len() || self.x_column == column {
            return false;
        }
        self.x_column = column;
        self.y_columns.retain(|selected| *selected != column);
        true
    }

    pub(crate) fn toggle_y_column(&mut self, column: usize) -> bool {
        if column >= self.columns.len() || !self.numeric_columns[column] || column == self.x_column
        {
            return false;
        }
        if let Some(position) = self
            .y_columns
            .iter()
            .position(|selected| *selected == column)
        {
            self.y_columns.remove(position);
        } else {
            self.y_columns.push(column);
        }
        true
    }

    pub(crate) fn series(&self) -> Result<Vec<ChartSeries>, ChartDataError> {
        if self.rows.is_empty() || self.columns.is_empty() {
            return Err(ChartDataError::Empty);
        }
        if !self.numeric_columns.iter().any(|numeric| *numeric) {
            return Err(ChartDataError::NoNumericColumns);
        }
        if self.y_columns.is_empty() {
            return Err(ChartDataError::NoSeriesSelected);
        }
        if self.chart_type == ChartType::Scatter && !self.numeric_columns[self.x_column] {
            return Err(ChartDataError::ScatterRequiresNumericX);
        }

        let categorical_axis = (!self.numeric_columns[self.x_column])
            .then(|| categorical_x_axis(&self.rows, self.x_column));
        let series = self
            .y_columns
            .iter()
            .filter_map(|column| {
                let points = match &categorical_axis {
                    Some(axis) => categorical_x_points(&self.rows, self.x_column, *column, axis),
                    None => numeric_x_points(&self.rows, self.x_column, *column),
                };
                (!points.is_empty()).then(|| ChartSeries {
                    name: self.columns[*column].clone(),
                    points,
                })
            })
            .collect::<Vec<_>>();
        if series.is_empty() {
            Err(ChartDataError::Empty)
        } else {
            Ok(series)
        }
    }
}

fn column_is_numeric(rows: &[Vec<Value>], column: usize) -> bool {
    let sample = rows
        .iter()
        .filter_map(|row| row.get(column))
        .filter(|value| !value.is_null())
        .take(10)
        .collect::<Vec<_>>();
    !sample.is_empty()
        && sample
            .iter()
            .filter(|value| numeric_value(value).is_some())
            .count()
            * 2
            > sample.len()
}

fn numeric_value(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn value_label(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "NULL".to_string(),
        Some(Value::String(value)) => value.clone(),
        Some(value) => value.to_string(),
    }
}

fn numeric_x_points(rows: &[Vec<Value>], x_column: usize, y_column: usize) -> Vec<ChartPoint> {
    rows.iter()
        .filter_map(|row| {
            let x = numeric_value(row.get(x_column)?)?;
            let y = numeric_value(row.get(y_column)?)?;
            Some(ChartPoint {
                label: value_label(row.get(x_column)),
                x,
                y,
            })
        })
        .collect()
}

fn categorical_x_axis(rows: &[Vec<Value>], x_column: usize) -> CategoricalAxis {
    let mut indexes = HashMap::new();
    let mut labels = Vec::new();
    for row in rows {
        let label = value_label(row.get(x_column));
        if !indexes.contains_key(&label) {
            indexes.insert(label.clone(), labels.len());
            labels.push(label);
        }
    }
    CategoricalAxis { labels, indexes }
}

fn categorical_x_points(
    rows: &[Vec<Value>],
    x_column: usize,
    y_column: usize,
    axis: &CategoricalAxis,
) -> Vec<ChartPoint> {
    let mut points = axis
        .labels
        .iter()
        .enumerate()
        .map(|(index, label)| ChartPoint {
            label: label.clone(),
            x: index as f64,
            y: 0.0,
        })
        .collect::<Vec<_>>();
    for row in rows {
        let Some(y) = row.get(y_column).and_then(numeric_value) else {
            continue;
        };
        let label = value_label(row.get(x_column));
        if let Some(index) = axis.indexes.get(&label) {
            points[*index].y += y;
        }
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: "text".to_string(),
            nullable: false,
            is_primary_key: false,
            default_value: None,
            comment: None,
        }
    }

    #[test]
    fn categorical_axes_aggregate_duplicate_labels() {
        let columns = vec![column("region"), column("revenue")];
        let rows = vec![
            vec![Value::from("East"), Value::from(7)],
            vec![Value::from("West"), Value::from(4)],
            vec![Value::from("East"), Value::from(5)],
        ];
        let model = ChartModel::new(&columns, &rows);

        assert_eq!(model.x_column(), 0);
        assert_eq!(model.y_columns(), &[1]);
        assert_eq!(
            model.series().unwrap()[0].points,
            vec![
                ChartPoint {
                    label: "East".to_string(),
                    x: 0.0,
                    y: 12.0,
                },
                ChartPoint {
                    label: "West".to_string(),
                    x: 1.0,
                    y: 4.0,
                },
            ]
        );
    }

    #[test]
    fn chart_type_switching_preserves_column_mapping() {
        let columns = vec![column("day"), column("orders")];
        let rows = vec![vec![Value::from("Mon"), Value::from(3)]];
        let mut model = ChartModel::new(&columns, &rows);

        for chart_type in ChartType::ALL {
            model.set_chart_type(chart_type);
            assert_eq!(model.chart_type(), chart_type);
            assert_eq!(model.x_column(), 0);
            assert_eq!(model.y_columns(), &[1]);
        }
    }

    #[test]
    fn numeric_x_values_are_not_aggregated() {
        let columns = vec![column("x"), column("y")];
        let rows = vec![
            vec![Value::from(1), Value::from(4)],
            vec![Value::from(1), Value::from(6)],
        ];
        let mut model = ChartModel::new(&columns, &rows);
        model.set_chart_type(ChartType::Scatter);

        assert_eq!(model.series().unwrap()[0].points.len(), 2);
    }

    #[test]
    fn nullable_series_share_categorical_positions() {
        let columns = vec![column("region"), column("orders"), column("revenue")];
        let rows = vec![
            vec![Value::from("East"), Value::from(3), Value::Null],
            vec![Value::from("West"), Value::Null, Value::from(7)],
        ];
        let model = ChartModel::new(&columns, &rows);

        let series = model.series().expect("chart series");
        assert_eq!(series[0].points.len(), 2);
        assert_eq!(series[1].points.len(), 2);
        assert_eq!(series[0].points[0].x, 0.0);
        assert_eq!(series[0].points[0].label, "East");
        assert_eq!(series[0].points[1].x, 1.0);
        assert_eq!(series[0].points[1].y, 0.0);
        assert_eq!(series[1].points[0].y, 0.0);
        assert_eq!(series[1].points[1].x, 1.0);
        assert_eq!(series[1].points[1].label, "West");
    }

    #[test]
    fn empty_and_non_numeric_results_have_explicit_errors() {
        let columns = vec![column("name")];
        assert_eq!(
            ChartModel::new(&columns, &[]).series(),
            Err(ChartDataError::Empty)
        );
        assert_eq!(
            ChartModel::new(&columns, &[vec![Value::from("Ada")]]).series(),
            Err(ChartDataError::NoNumericColumns)
        );
    }

    #[test]
    fn scatter_requires_a_numeric_x_column() {
        let columns = vec![column("name"), column("score")];
        let rows = vec![vec![Value::from("Ada"), Value::from(9)]];
        let mut model = ChartModel::new(&columns, &rows);
        model.set_chart_type(ChartType::Scatter);

        assert_eq!(model.series(), Err(ChartDataError::ScatterRequiresNumericX));
    }
}
