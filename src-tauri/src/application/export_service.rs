use rust_xlsxwriter::{Format, Workbook};
use serde_json::Value;

use crate::tasks::{NewTask, TaskManager, TaskOutcome};

use super::{QueryService, QueryTarget};

/// Where the rows to export come from.
///
/// `Sql` keeps large query results inside the export workflow; `Rows` preserves
/// an already-materialized selection exactly as supplied by the caller.
#[derive(Debug)]
pub enum ExportSource {
    Sql {
        sql: String,
    },
    Rows {
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
    },
}

#[derive(Debug)]
pub struct CsvOptions {
    pub delimiter: String,
    pub include_header: bool,
    pub quote_all: bool,
    pub null_value: String,
    pub crlf: bool,
    pub bom: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonLayout {
    Objects,
    Arrays,
}

#[derive(Debug)]
pub struct JsonOptions {
    pub layout: JsonLayout,
    pub pretty: bool,
}

#[derive(Debug)]
pub struct XlsxOptions {
    pub include_header: bool,
    pub sheet_name: String,
}

#[derive(Debug)]
pub enum ExportFormat {
    Csv(CsvOptions),
    Json(JsonOptions),
    Xlsx(XlsxOptions),
}

#[derive(Clone)]
pub struct ExportService {
    queries: QueryService,
    tasks: TaskManager,
}

impl ExportService {
    pub(super) fn new(queries: QueryService, tasks: TaskManager) -> Self {
        Self { queries, tasks }
    }

    pub(crate) async fn start_export(
        &self,
        target: QueryTarget,
        source: ExportSource,
        format: ExportFormat,
        output_path: String,
    ) -> Result<String, String> {
        let service = self.clone();
        let name = std::path::Path::new(&output_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("export")
            .to_string();
        Ok(self
            .tasks
            .spawn(
                NewTask {
                    name: format!("Export {name}"),
                    initial_message: "Preparing export...".to_string(),
                },
                move |task| async move {
                    if task.is_cancelled() {
                        return TaskOutcome::Cancelled("Export cancelled".to_string());
                    }
                    task.progress(0.1, "Loading export rows...").await;
                    let rows = match service.materialize_target(&target, source).await {
                        Ok(rows) => rows,
                        Err(error) => return TaskOutcome::Failed(error),
                    };
                    if task.is_cancelled() {
                        return TaskOutcome::Cancelled(
                            "Export cancelled before the output file was written".to_string(),
                        );
                    }
                    let count = rows.1.len();
                    task.progress(0.7, format!("Writing {count} row(s)..."))
                        .await;
                    match write_export(rows.0, rows.1, format, output_path).await {
                        Ok(()) => TaskOutcome::Completed(format!("Exported {count} row(s)")),
                        Err(error) => TaskOutcome::Failed(error),
                    }
                },
            )
            .await)
    }

    pub async fn export(
        &self,
        connection_id: &str,
        database: &str,
        source: ExportSource,
        format: ExportFormat,
        output_path: String,
    ) -> Result<usize, String> {
        let (headers, rows): (Vec<String>, Vec<Vec<Value>>) = match source {
            ExportSource::Rows { columns, rows } => (columns, rows),
            ExportSource::Sql { sql } => {
                let result = self.queries.execute(connection_id, database, &sql).await?;
                into_export_rows(result)
            }
        };

        let count = rows.len();
        write_export(headers, rows, format, output_path).await?;
        Ok(count)
    }

    async fn materialize_target(
        &self,
        target: &QueryTarget,
        source: ExportSource,
    ) -> Result<(Vec<String>, Vec<Vec<Value>>), String> {
        match source {
            ExportSource::Rows { columns, rows } => Ok((columns, rows)),
            ExportSource::Sql { sql } => {
                let result = self.queries.execute_export_query(target, &sql).await?;
                Ok(into_export_rows(result))
            }
        }
    }
}

fn into_export_rows(result: crate::db::QueryResult) -> (Vec<String>, Vec<Vec<Value>>) {
    let headers = result
        .columns
        .into_iter()
        .map(|column| column.name)
        .collect();
    (headers, result.rows)
}

async fn write_export(
    headers: Vec<String>,
    rows: Vec<Vec<Value>>,
    format: ExportFormat,
    output_path: String,
) -> Result<(), String> {
    // Serialization + file IO is CPU/IO bound — keep it off the async runtime.
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        match format {
            ExportFormat::Csv(options) => {
                let body = write_csv(&headers, &rows, &options);
                std::fs::write(&output_path, body).map_err(|e| format!("写入文件失败: {}", e))
            }
            ExportFormat::Json(options) => {
                let body = write_json(&headers, &rows, &options)?;
                std::fs::write(&output_path, body).map_err(|e| format!("写入文件失败: {}", e))
            }
            ExportFormat::Xlsx(options) => write_xlsx(&headers, &rows, &options, &output_path),
        }
    })
    .await
    .map_err(|e| format!("导出任务失败: {}", e))?
}

fn csv_escape(s: &str, opts: &CsvOptions) -> String {
    let needs_quote = opts.quote_all
        || s.contains(&opts.delimiter)
        || s.contains('"')
        || s.contains('\n')
        || s.contains('\r');
    if needs_quote {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn csv_cell(v: &Value, opts: &CsvOptions) -> String {
    match v {
        Value::Null => opts.null_value.clone(),
        Value::Bool(b) => csv_escape(&b.to_string(), opts),
        Value::Number(n) => csv_escape(&n.to_string(), opts),
        Value::String(s) => csv_escape(s, opts),
        other => csv_escape(&other.to_string(), opts),
    }
}

fn write_csv(headers: &[String], rows: &[Vec<Value>], opts: &CsvOptions) -> String {
    let eol = if opts.crlf { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = Vec::with_capacity(rows.len() + 1);
    if opts.include_header {
        lines.push(
            headers
                .iter()
                .map(|h| csv_escape(h, opts))
                .collect::<Vec<_>>()
                .join(&opts.delimiter),
        );
    }
    for row in rows {
        lines.push(
            row.iter()
                .map(|v| csv_cell(v, opts))
                .collect::<Vec<_>>()
                .join(&opts.delimiter),
        );
    }

    let mut out = String::new();
    if opts.bom {
        out.push('\u{FEFF}');
    }
    if !lines.is_empty() {
        out.push_str(&lines.join(eol));
        out.push_str(eol);
    }
    out
}

fn write_json(
    headers: &[String],
    rows: &[Vec<Value>],
    opts: &JsonOptions,
) -> Result<String, String> {
    if opts.layout == JsonLayout::Arrays {
        return if opts.pretty {
            serde_json::to_string_pretty(rows)
        } else {
            serde_json::to_string(rows)
        }
        .map_err(|error| format!("JSON 序列化失败: {error}"));
    }

    // "objects" layout — built by hand so column order is preserved (serde_json's
    // default Map would otherwise sort keys alphabetically). Cell values still go
    // through serde_json for correct escaping; nested values stay on one line.
    let mut out = String::from("[");
    for (ri, row) in rows.iter().enumerate() {
        if ri > 0 {
            out.push(',');
        }
        if opts.pretty {
            out.push_str("\n  ");
        }
        out.push('{');
        for (ci, header) in headers.iter().enumerate() {
            if ci > 0 {
                out.push(',');
            }
            if opts.pretty {
                out.push_str("\n    ");
            }
            let key = serde_json::to_string(header)
                .map_err(|error| format!("JSON 字段名序列化失败: {error}"))?;
            let val = serde_json::to_string(row.get(ci).unwrap_or(&Value::Null))
                .map_err(|error| format!("JSON 值序列化失败: {error}"))?;
            out.push_str(&key);
            out.push(':');
            if opts.pretty {
                out.push(' ');
            }
            out.push_str(&val);
        }
        if opts.pretty && !headers.is_empty() {
            out.push_str("\n  ");
        }
        out.push('}');
    }
    if opts.pretty && !rows.is_empty() {
        out.push('\n');
    }
    out.push(']');
    Ok(out)
}

/// Excel sheet names: max 31 chars, and `: \ / ? * [ ]` are forbidden.
fn sanitize_sheet_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            ':' | '\\' | '/' | '?' | '*' | '[' | ']' => '_',
            other => other,
        })
        .take(31)
        .collect();
    if cleaned.is_empty() {
        "Sheet1".to_string()
    } else {
        cleaned
    }
}

fn write_xlsx(
    headers: &[String],
    rows: &[Vec<Value>],
    opts: &XlsxOptions,
    path: &str,
) -> Result<(), String> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet
        .set_name(sanitize_sheet_name(&opts.sheet_name))
        .map_err(|e| e.to_string())?;

    let mut data_start: u32 = 0;
    if opts.include_header {
        let header_format = Format::new().set_bold();
        for (c, header) in headers.iter().enumerate() {
            worksheet
                .write_string_with_format(0, c as u16, header, &header_format)
                .map_err(|e| e.to_string())?;
        }
        data_start = 1;
    }

    for (r, row) in rows.iter().enumerate() {
        let xr = data_start + r as u32;
        for (c, value) in row.iter().enumerate() {
            let xc = c as u16;
            match value {
                Value::Null => {}
                Value::Bool(b) => {
                    worksheet
                        .write_boolean(xr, xc, *b)
                        .map_err(|e| e.to_string())?;
                }
                Value::Number(n) => {
                    // Keep integers that don't fit f64 exactly as text to avoid precision loss.
                    if let Some(i) = n.as_i64() {
                        if (i as f64) as i64 == i {
                            worksheet
                                .write_number(xr, xc, i as f64)
                                .map_err(|e| e.to_string())?;
                        } else {
                            worksheet
                                .write_string(xr, xc, i.to_string())
                                .map_err(|e| e.to_string())?;
                        }
                    } else if let Some(f) = n.as_f64() {
                        worksheet
                            .write_number(xr, xc, f)
                            .map_err(|e| e.to_string())?;
                    } else {
                        worksheet
                            .write_string(xr, xc, n.to_string())
                            .map_err(|e| e.to_string())?;
                    }
                }
                Value::String(s) => {
                    worksheet
                        .write_string(xr, xc, s)
                        .map_err(|e| e.to_string())?;
                }
                other => {
                    worksheet
                        .write_string(xr, xc, other.to_string())
                        .map_err(|e| e.to_string())?;
                }
            }
        }
    }

    if opts.include_header && !headers.is_empty() {
        let last_row = data_start + rows.len() as u32;
        let last_row = last_row.saturating_sub(1);
        let last_col = (headers.len() as u16).saturating_sub(1);
        worksheet
            .autofilter(0, 0, last_row, last_col)
            .map_err(|e| e.to_string())?;
        worksheet
            .set_freeze_panes(1, 0)
            .map_err(|e| e.to_string())?;
    }
    worksheet.autofit();

    workbook.save(path).map_err(|e| e.to_string())?;
    Ok(())
}
