use std::io::Write;

use rust_xlsxwriter::{cell_autofit_width, Format, Workbook};
use serde_json::Value;
use tokio::sync::mpsc::Receiver;

use crate::application::atomic_output::AtomicOutput;
use crate::tasks::TaskContext;

use super::{CsvOptions, ExportEvent, ExportFormat, JsonLayout, JsonOptions, XlsxOptions};

pub(super) fn write_export(
    mut receiver: Receiver<ExportEvent>,
    format: ExportFormat,
    output_path: String,
    task: Option<TaskContext>,
) -> Result<usize, String> {
    let mut output = AtomicOutput::new(output_path).map_err(|error| error.to_string())?;
    let headers = match receiver.blocking_recv() {
        Some(ExportEvent::Columns(headers)) => headers,
        _ => return Err("Export ended before column metadata arrived".to_string()),
    };
    let mut rows = ExportRows {
        receiver,
        task,
        finished: false,
    };
    let count = match format {
        ExportFormat::Csv(options) => write_csv(&mut output, &headers, &mut rows, &options),
        ExportFormat::Json(options) => write_json(&mut output, &headers, &mut rows, &options),
        ExportFormat::Xlsx(options) => write_xlsx(&mut output, &headers, &mut rows, &options),
    }
    .map_err(|error| error.to_string())?;
    if rows.task.as_ref().is_some_and(TaskContext::is_cancelled) {
        return Err("Export cancelled".to_string());
    }
    output
        .commit()
        .map_err(|error| format!("写入文件失败: {error}"))?;
    Ok(count)
}

struct ExportRows {
    receiver: Receiver<ExportEvent>,
    task: Option<TaskContext>,
    finished: bool,
}

impl Iterator for ExportRows {
    type Item = anyhow::Result<Vec<Value>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        if self.task.as_ref().is_some_and(TaskContext::is_cancelled) {
            self.finished = true;
            return Some(Err(anyhow::anyhow!("Export cancelled")));
        }
        match self.receiver.blocking_recv() {
            Some(ExportEvent::Row(row)) => Some(Ok(row)),
            Some(ExportEvent::Finish) => {
                self.finished = true;
                None
            }
            _ => {
                self.finished = true;
                Some(Err(anyhow::anyhow!("Export ended before query completion")))
            }
        }
    }
}

fn csv_escape(text: &str, options: &CsvOptions) -> String {
    if options.quote_all
        || text.contains(&options.delimiter)
        || text.contains('"')
        || text.contains('\n')
        || text.contains('\r')
    {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn csv_cell(value: &Value, options: &CsvOptions) -> String {
    match value {
        Value::Null => options.null_value.clone(),
        Value::String(text) => csv_escape(text, options),
        value => csv_escape(&value.to_string(), options),
    }
}

fn write_csv(
    output: &mut impl Write,
    headers: &[String],
    rows: &mut impl Iterator<Item = anyhow::Result<Vec<Value>>>,
    options: &CsvOptions,
) -> anyhow::Result<usize> {
    let eol = if options.crlf { "\r\n" } else { "\n" };
    if options.bom {
        output.write_all("\u{FEFF}".as_bytes())?;
    }
    if options.include_header {
        for (index, header) in headers.iter().enumerate() {
            if index > 0 {
                output.write_all(options.delimiter.as_bytes())?;
            }
            output.write_all(csv_escape(header, options).as_bytes())?;
        }
        output.write_all(eol.as_bytes())?;
    }
    let mut count = 0;
    for row in rows {
        for (index, value) in row?.iter().enumerate() {
            if index > 0 {
                output.write_all(options.delimiter.as_bytes())?;
            }
            output.write_all(csv_cell(value, options).as_bytes())?;
        }
        output.write_all(eol.as_bytes())?;
        count += 1;
    }
    Ok(count)
}

fn write_json(
    output: &mut impl Write,
    headers: &[String],
    rows: &mut impl Iterator<Item = anyhow::Result<Vec<Value>>>,
    options: &JsonOptions,
) -> anyhow::Result<usize> {
    output.write_all(b"[")?;
    let mut count = 0;
    for row in rows {
        let row = row?;
        if count > 0 {
            output.write_all(b",")?;
        }
        if options.pretty {
            output.write_all(b"\n  ")?;
        }
        if options.layout == JsonLayout::Arrays {
            if options.pretty {
                let serialized = serde_json::to_string_pretty(&row)?;
                for (index, line) in serialized.split('\n').enumerate() {
                    if index > 0 {
                        output.write_all(b"\n  ")?;
                    }
                    output.write_all(line.as_bytes())?;
                }
            } else {
                serde_json::to_writer(&mut *output, &row)?;
            }
        } else {
            output.write_all(b"{")?;
            for (index, header) in headers.iter().enumerate() {
                if index > 0 {
                    output.write_all(b",")?;
                }
                if options.pretty {
                    output.write_all(b"\n    ")?;
                }
                serde_json::to_writer(&mut *output, header)?;
                output.write_all(if options.pretty { b": " } else { b":" })?;
                serde_json::to_writer(&mut *output, row.get(index).unwrap_or(&Value::Null))?;
            }
            if options.pretty && !headers.is_empty() {
                output.write_all(b"\n  ")?;
            }
            output.write_all(b"}")?;
        }
        count += 1;
    }
    if options.pretty && count > 0 {
        output.write_all(b"\n")?;
    }
    output.write_all(b"]")?;
    Ok(count)
}

fn sanitize_sheet_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| match character {
            ':' | '\\' | '/' | '?' | '*' | '[' | ']' => '_',
            character => character,
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
    output: &mut AtomicOutput,
    headers: &[String],
    rows: &mut impl Iterator<Item = anyhow::Result<Vec<Value>>>,
    options: &XlsxOptions,
) -> anyhow::Result<usize> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet_with_constant_memory();
    worksheet.set_name(sanitize_sheet_name(&options.sheet_name))?;
    let data_start = u32::from(options.include_header);
    let mut widths = Vec::<u32>::new();
    if options.include_header {
        let header_format = Format::new().set_bold();
        for (column, header) in headers.iter().enumerate() {
            worksheet.write_string_with_format(
                0,
                u16::try_from(column)?,
                header,
                &header_format,
            )?;
            widths.push(cell_autofit_width(header).saturating_add(16));
        }
    }
    let mut count = 0;
    for row in rows {
        let row = row?;
        let row_index = data_start
            .checked_add(u32::try_from(count)?)
            .ok_or_else(|| anyhow::anyhow!("Excel row limit exceeded"))?;
        widths.resize(widths.len().max(row.len()), 0);
        for (column, value) in row.iter().enumerate() {
            let column_index = u16::try_from(column)?;
            let width = match value {
                Value::Null => 0,
                Value::Bool(value) => {
                    worksheet.write_boolean(row_index, column_index, *value)?;
                    if *value {
                        38
                    } else {
                        43
                    }
                }
                Value::Number(number) => {
                    if let Some(integer) = number.as_i64() {
                        if (integer as f64) as i64 == integer {
                            worksheet.write_number(row_index, column_index, integer as f64)?;
                        } else {
                            worksheet.write_string(row_index, column_index, integer.to_string())?;
                        }
                    } else if let Some(number) = number.as_f64() {
                        worksheet.write_number(row_index, column_index, number)?;
                    } else {
                        worksheet.write_string(row_index, column_index, number.to_string())?;
                    }
                    cell_autofit_width(&number.to_string())
                }
                Value::String(text) => {
                    worksheet.write_string(row_index, column_index, text)?;
                    text.lines().map(cell_autofit_width).max().unwrap_or(0)
                }
                value => {
                    let text = value.to_string();
                    worksheet.write_string(row_index, column_index, &text)?;
                    cell_autofit_width(&text)
                }
            };
            widths[column] = widths[column].max(width);
        }
        count += 1;
    }
    if options.include_header && !headers.is_empty() {
        worksheet.autofilter(
            0,
            0,
            u32::try_from(count)?,
            u16::try_from(headers.len() - 1)?,
        )?;
        worksheet.set_freeze_panes(1, 0)?;
    }
    for (column, width) in widths.into_iter().enumerate() {
        if width > 0 {
            worksheet.set_column_width_pixels(u16::try_from(column)?, width.min(1790))?;
        }
    }
    workbook.save_to_writer(output)?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn csv_preserves_escaping_bom_delimiter_and_line_endings() {
        let mut output = Vec::new();
        let count = write_csv(
            &mut output,
            &["z;\"".to_string(), "a".to_string()],
            &mut vec![Ok(vec![json!("line\nbreak"), Value::Null])].into_iter(),
            &CsvOptions {
                delimiter: ";".to_string(),
                include_header: true,
                quote_all: false,
                null_value: "NULL".to_string(),
                crlf: true,
                bom: true,
            },
        )
        .unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\u{FEFF}\"z;\"\"\";a\r\n\"line\nbreak\";NULL\r\n"
        );
    }

    #[test]
    fn pretty_json_arrays_match_serde_for_nested_values() {
        let values = vec![vec![json!({"nested": [1, "two"]}), Value::Null], Vec::new()];
        let mut output = Vec::new();
        write_json(
            &mut output,
            &[],
            &mut values.clone().into_iter().map(Ok),
            &JsonOptions {
                layout: JsonLayout::Arrays,
                pretty: true,
            },
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            serde_json::to_string_pretty(&values).unwrap()
        );
    }

    #[test]
    fn json_objects_preserve_column_order_and_missing_cells() {
        let mut output = Vec::new();
        write_json(
            &mut output,
            &["z".to_string(), "a".to_string()],
            &mut vec![Ok(vec![json!("quoted\"")])].into_iter(),
            &JsonOptions {
                layout: JsonLayout::Objects,
                pretty: true,
            },
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "[\n  {\n    \"z\": \"quoted\\\"\",\n    \"a\": null\n  }\n]"
        );
    }

    #[test]
    fn a_query_failure_after_rows_preserves_the_destination() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("export.json");
        std::fs::write(&path, "existing output").unwrap();
        let (sender, receiver) = tokio::sync::mpsc::channel(4);
        sender
            .blocking_send(ExportEvent::Columns(vec!["value".to_string()]))
            .unwrap();
        sender
            .blocking_send(ExportEvent::Row(vec![json!(1)]))
            .unwrap();
        drop(sender);
        let result = write_export(
            receiver,
            ExportFormat::Json(JsonOptions {
                layout: JsonLayout::Objects,
                pretty: false,
            }),
            path.to_string_lossy().into_owned(),
            None,
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing output");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
