use std::{
    collections::HashMap,
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use serde_json::Value;

#[cfg(test)]
use crate::db::ColumnInfo;
use crate::db::StatementResult;

use super::{GridLoadError, GridLoadRequest, GridPage, GridQuery, GridService, QueryTarget};
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
        cancelled: &AtomicBool,
    ) -> Result<Option<ChartTableData>, GridLoadError> {
        collect_table_pages(cancelled, |page_number| {
            let request = GridLoadRequest::for_chart_page(
                target.clone(),
                table.clone(),
                query.clone(),
                page_number,
                CHART_PAGE_SIZE,
            );
            async move { self.grids.load(&request).await }
        })
        .await
    }
}

async fn collect_table_pages<F, Fut>(
    cancelled: &AtomicBool,
    mut load_page: F,
) -> Result<Option<ChartTableData>, GridLoadError>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<GridPage, GridLoadError>>,
{
    let mut page_number = 1_u32;
    let mut columns = Vec::new();
    let mut rows = Vec::new();
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(None);
        }
        // Shared driver streams must finish before their session can be reused.
        let page = load_page(page_number).await;
        if cancelled.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let page = page?;
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
    Ok(Some(ChartTableData { columns, rows }))
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

#[derive(Debug)]
pub(crate) struct ChartModel {
    columns: Vec<String>,
    numeric_columns: Vec<bool>,
    data: ChartRows,
    chart_type: ChartType,
    x_column: usize,
    y_columns: Vec<usize>,
}

#[derive(Debug)]
enum ChartRows {
    Owned(Vec<Vec<Value>>),
    Statement(Arc<StatementResult>),
}

impl ChartRows {
    fn rows(&self) -> &[Vec<Value>] {
        match self {
            Self::Owned(rows) => rows,
            Self::Statement(result) => &result.rows,
        }
    }
}

impl ChartModel {
    #[cfg(test)]
    pub(crate) fn new(columns: &[ColumnInfo], rows: Vec<Vec<Value>>) -> Self {
        Self::from_names(
            columns.iter().map(|column| column.name.clone()).collect(),
            rows,
        )
    }

    pub(crate) fn from_names(columns: Vec<String>, rows: Vec<Vec<Value>>) -> Self {
        Self::from_data(columns, ChartRows::Owned(rows))
    }

    pub(crate) fn from_statement(result: Arc<StatementResult>) -> Self {
        let columns = result
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect();
        Self::from_data(columns, ChartRows::Statement(result))
    }

    fn from_data(columns: Vec<String>, data: ChartRows) -> Self {
        let numeric_columns = (0..columns.len())
            .map(|column| column_is_numeric(data.rows(), column))
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
            data,
            chart_type: ChartType::Bar,
            x_column,
            y_columns,
        }
    }

    pub(crate) fn replace_data(&mut self, columns: Vec<String>, rows: Vec<Vec<Value>>) {
        self.replace_model(Self::from_names(columns, rows));
    }

    pub(crate) fn replace_statement(&mut self, result: Arc<StatementResult>) {
        self.replace_model(Self::from_statement(result));
    }

    pub(crate) fn release_data(&mut self) {
        self.data = ChartRows::Owned(Vec::new());
    }

    fn replace_model(&mut self, mut replacement: Self) {
        let chart_type = self.chart_type;
        let x_name = self.columns.get(self.x_column).cloned();
        let y_names = self
            .y_columns
            .iter()
            .filter_map(|column| self.columns.get(*column).cloned())
            .collect::<Vec<_>>();
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
        let rows = self.data.rows();
        if rows.is_empty() || self.columns.is_empty() {
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

        let categorical_axis =
            (!self.numeric_columns[self.x_column]).then(|| categorical_x_axis(rows, self.x_column));
        let series = self
            .y_columns
            .iter()
            .filter_map(|column| {
                let points = match &categorical_axis {
                    Some(axis) => categorical_x_points(rows, self.x_column, *column, axis),
                    None => numeric_x_points(rows, self.x_column, *column),
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

    #[tokio::test]
    async fn cancelled_chart_does_not_fetch_the_first_page() {
        let cancelled = AtomicBool::new(true);
        let mut pages_loaded = 0;
        let result = collect_table_pages(&cancelled, |_| {
            pages_loaded += 1;
            std::future::ready(Err(GridLoadError::Query("unexpected fetch".to_string())))
        })
        .await
        .unwrap();

        assert!(result.is_none());
        assert_eq!(pages_loaded, 0);
    }

    #[tokio::test]
    async fn cancellation_waits_for_the_current_page_and_skips_remaining_pages() {
        let cancelled = AtomicBool::new(false);
        let mut pages_loaded = 0;
        let pages_completed = std::cell::Cell::new(0);
        let result = collect_table_pages(&cancelled, |page| {
            pages_loaded += 1;
            let cancelled = &cancelled;
            let pages_completed = &pages_completed;
            async move {
                if page == 2 {
                    cancelled.store(true, Ordering::Relaxed);
                }
                tokio::task::yield_now().await;
                pages_completed.set(pages_completed.get() + 1);
                Ok(GridPage::new(
                    vec![column("id")],
                    vec![vec![Value::from(page)]; CHART_PAGE_SIZE as usize],
                    None,
                )
                .unwrap())
            }
        })
        .await
        .unwrap();

        assert!(result.is_none());
        assert_eq!(pages_loaded, 2);
        assert_eq!(pages_completed.get(), 2);
    }

    #[tokio::test]
    async fn active_chart_keeps_every_matching_page_in_order() {
        let cancelled = AtomicBool::new(false);
        let mut pages_loaded = 0;
        let result = collect_table_pages(&cancelled, |page| {
            pages_loaded += 1;
            let count = if page < 3 {
                CHART_PAGE_SIZE as usize
            } else {
                1
            };
            std::future::ready(Ok(GridPage::new(
                vec![column("id")],
                vec![vec![Value::from(page)]; count],
                None,
            )
            .unwrap()))
        })
        .await
        .unwrap()
        .unwrap();

        assert_eq!(pages_loaded, 3);
        assert_eq!(result.rows.len(), 2_001);
        assert_eq!(result.rows[0][0], Value::from(1));
        assert_eq!(result.rows[1_000][0], Value::from(2));
        assert_eq!(result.rows[2_000][0], Value::from(3));
    }

    #[test]
    fn categorical_axes_aggregate_duplicate_labels() {
        let columns = vec![column("region"), column("revenue")];
        let rows = vec![
            vec![Value::from("East"), Value::from(7)],
            vec![Value::from("West"), Value::from(4)],
            vec![Value::from("East"), Value::from(5)],
        ];
        let model = ChartModel::new(&columns, rows);

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
        let mut model = ChartModel::new(&columns, rows);

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
        let mut model = ChartModel::new(&columns, rows);
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
        let model = ChartModel::new(&columns, rows);

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
            ChartModel::new(&columns, vec![]).series(),
            Err(ChartDataError::Empty)
        );
        assert_eq!(
            ChartModel::new(&columns, vec![vec![Value::from("Ada")]]).series(),
            Err(ChartDataError::NoNumericColumns)
        );
    }

    #[test]
    fn scatter_requires_a_numeric_x_column() {
        let columns = vec![column("name"), column("score")];
        let rows = vec![vec![Value::from("Ada"), Value::from(9)]];
        let mut model = ChartModel::new(&columns, rows);
        model.set_chart_type(ChartType::Scatter);

        assert_eq!(model.series(), Err(ChartDataError::ScatterRequiresNumericX));
    }

    #[test]
    fn table_data_moves_into_the_model_without_copying_rows() {
        let rows = vec![vec![Value::from("East"), Value::from(7)]];
        let storage = rows.as_ptr();
        let model = ChartModel::new(&[column("region"), column("revenue")], rows);

        assert_eq!(model.data.rows().as_ptr(), storage);
        assert_eq!(model.series().unwrap()[0].points[0].y, 7.0);
    }

    #[test]
    fn query_data_is_shared_and_released_without_losing_mappings() {
        let result = Arc::new(StatementResult::from_query_result(
            "SELECT x, y, label".to_string(),
            crate::db::QueryResult {
                columns: vec![column("x"), column("y"), column("label")],
                rows: vec![vec![Value::from(1), Value::from(7), Value::from("East")]],
                ..Default::default()
            },
        ));
        let mut model = ChartModel::from_statement(result.clone());
        model.set_x_column(0);
        model.set_chart_type(ChartType::Scatter);
        assert_eq!(Arc::strong_count(&result), 2);
        assert_eq!(model.data.rows().as_ptr(), result.rows.as_ptr());

        model.release_data();
        assert_eq!(Arc::strong_count(&result), 1);
        assert_eq!(model.series(), Err(ChartDataError::Empty));

        model.replace_data(
            vec!["label".to_string(), "y".to_string(), "x".to_string()],
            vec![vec![Value::from("West"), Value::from(8), Value::from(2)]],
        );
        assert_eq!(model.chart_type(), ChartType::Scatter);
        assert_eq!(model.x_column(), 2);
        assert_eq!(model.y_columns(), &[1]);
        assert_eq!(model.series().unwrap()[0].points[0].x, 2.0);
    }
}
