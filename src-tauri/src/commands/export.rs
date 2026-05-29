use rust_xlsxwriter::{Format, Workbook};
use serde::Deserialize;
use serde_json::Value;
use tauri::State;

use crate::state::AppState;

/// Where the rows to export come from.
///
/// - `Sql`: run the query in Rust and stream the result straight to the file,
///   avoiding shipping the whole dataset across the IPC bridge (used for table
///   "all rows" / "range" exports).
/// - `Rows`: rows already held by the frontend (current page, or an arbitrary
///   query result in the SQL editor), passed through as-is.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ExportSource {
    Sql { sql: String },
    Rows {
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvOptions {
    pub delimiter: String,
    pub include_header: bool,
    pub quote_all: bool,
    pub null_value: String,
    pub crlf: bool,
    pub bom: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonOptions {
    pub layout: String, // "objects" | "arrays"
    pub pretty: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XlsxOptions {
    pub include_header: bool,
    pub sheet_name: String,
}

#[derive(Deserialize)]
pub struct ExportOptions {
    pub csv: CsvOptions,
    pub json: JsonOptions,
    pub xlsx: XlsxOptions,
}

#[tauri::command]
pub async fn export_data(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    source: ExportSource,
    format: String,
    options: ExportOptions,
    output_path: String,
) -> Result<usize, String> {
    // Resolve the rows to export.
    let (headers, rows): (Vec<String>, Vec<Vec<Value>>) = match source {
        ExportSource::Rows { columns, rows } => (columns, rows),
        ExportSource::Sql { sql } => {
            let connections = state.connections.lock().await;
            let driver = connections.get(&connection_id).ok_or("连接不存在")?;
            let result = driver
                .execute_query(&database, &sql)
                .await
                .map_err(|e| format!("查询失败: {}", e))?;
            let headers = result.columns.iter().map(|c| c.name.clone()).collect();
            (headers, result.rows)
        }
    };

    let count = rows.len();

    // Serialization + file IO is CPU/IO bound — keep it off the async runtime.
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        match format.as_str() {
            "csv" => {
                let body = write_csv(&headers, &rows, &options.csv);
                std::fs::write(&output_path, body).map_err(|e| format!("写入文件失败: {}", e))
            }
            "json" => {
                let body = write_json(&headers, &rows, &options.json);
                std::fs::write(&output_path, body).map_err(|e| format!("写入文件失败: {}", e))
            }
            "xlsx" => write_xlsx(&headers, &rows, &options.xlsx, &output_path),
            other => Err(format!("不支持的导出格式: {}", other)),
        }
    })
    .await
    .map_err(|e| format!("导出任务失败: {}", e))??;

    Ok(count)
}

// ----- CSV -----

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

// ----- JSON -----

fn write_json(headers: &[String], rows: &[Vec<Value>], opts: &JsonOptions) -> String {
    if opts.layout == "arrays" {
        return if opts.pretty {
            serde_json::to_string_pretty(rows).unwrap_or_else(|_| "[]".to_string())
        } else {
            serde_json::to_string(rows).unwrap_or_else(|_| "[]".to_string())
        };
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
            let key = serde_json::to_string(header).unwrap_or_else(|_| "\"\"".to_string());
            let val = serde_json::to_string(row.get(ci).unwrap_or(&Value::Null))
                .unwrap_or_else(|_| "null".to_string());
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
    out
}

// ----- XLSX (rust_xlsxwriter) -----

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
                Value::Null => {} // leave the cell blank
                Value::Bool(b) => {
                    worksheet.write_boolean(xr, xc, *b).map_err(|e| e.to_string())?;
                }
                Value::Number(n) => {
                    // Keep integers that don't fit f64 exactly as text to avoid precision loss.
                    if let Some(i) = n.as_i64() {
                        if (i as f64) as i64 == i {
                            worksheet.write_number(xr, xc, i as f64).map_err(|e| e.to_string())?;
                        } else {
                            worksheet.write_string(xr, xc, i.to_string()).map_err(|e| e.to_string())?;
                        }
                    } else if let Some(f) = n.as_f64() {
                        worksheet.write_number(xr, xc, f).map_err(|e| e.to_string())?;
                    } else {
                        worksheet.write_string(xr, xc, n.to_string()).map_err(|e| e.to_string())?;
                    }
                }
                Value::String(s) => {
                    worksheet.write_string(xr, xc, s).map_err(|e| e.to_string())?;
                }
                other => {
                    worksheet.write_string(xr, xc, other.to_string()).map_err(|e| e.to_string())?;
                }
            }
        }
    }

    // With a header row, add Excel's filter/sort buttons and freeze the header.
    if opts.include_header && !headers.is_empty() {
        let last_row = data_start + rows.len() as u32;
        let last_row = last_row.saturating_sub(1);
        let last_col = (headers.len() as u16).saturating_sub(1);
        worksheet
            .autofilter(0, 0, last_row, last_col)
            .map_err(|e| e.to_string())?;
        worksheet.set_freeze_panes(1, 0).map_err(|e| e.to_string())?;
    }
    worksheet.autofit();

    workbook.save(path).map_err(|e| e.to_string())?;
    Ok(())
}
