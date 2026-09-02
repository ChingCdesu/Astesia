use serde_json::Value;

pub(super) fn format_grid_value(value: &Value) -> String {
    match value {
        Value::Null => "\\N".to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) | Value::Bool(_) | Value::Number(_) => value.to_string(),
    }
}

pub(super) fn format_grid_tsv(rows: Vec<Vec<String>>) -> String {
    rows.into_iter()
        .map(|row| {
            row.into_iter()
                .map(|value| {
                    if value.contains(['\t', '\n', '\r', '"']) {
                        format!("\"{}\"", value.replace('"', "\"\""))
                    } else {
                        value
                    }
                })
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
