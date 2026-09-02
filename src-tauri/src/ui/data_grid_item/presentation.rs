use super::*;

pub(super) fn edit_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
        value => value.to_string(),
    }
}

pub(super) fn cell_editor_modified(
    initial_value: Option<&Value>,
    initial_text: &str,
    current_text: &str,
    null_requested: bool,
) -> bool {
    if null_requested {
        !initial_value.is_some_and(Value::is_null)
    } else {
        match initial_value {
            Some(value) if value.is_null() => true,
            Some(_) => current_text != initial_text,
            None => !current_text.is_empty(),
        }
    }
}

pub(super) fn grid_paste_assignments(
    page: &GridPage,
    anchor: Option<GridCell>,
    input: &str,
) -> Result<Vec<(GridCell, Value)>, GridPasteError> {
    let anchor = anchor.ok_or(GridPasteError::NoTarget)?;
    let delimiter = if input.contains('\t') { b'\t' } else { b',' };
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(input.as_bytes());
    let rows = reader
        .records()
        .map(|record| {
            record
                .map(|record| record.iter().map(str::to_string).collect::<Vec<_>>())
                .map_err(|error| GridPasteError::InvalidDelimited(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if rows.is_empty() || rows.iter().all(Vec::is_empty) {
        return Err(GridPasteError::Empty);
    }

    let header_columns = rows.first().and_then(|row| {
        let columns = row
            .iter()
            .map(|name| page.columns.iter().position(|column| column.name == *name))
            .collect::<Option<Vec<_>>>()?;
        let mut unique = columns.clone();
        unique.sort_unstable();
        unique.dedup();
        (unique.len() == columns.len()).then_some(columns)
    });
    let (columns, data_rows) = if let Some(columns) = header_columns {
        (columns, &rows[1..])
    } else {
        let width = rows.first().map_or(0, Vec::len);
        (
            (anchor.column..anchor.column.saturating_add(width)).collect(),
            rows.as_slice(),
        )
    };
    if data_rows.is_empty() || columns.is_empty() {
        return Err(GridPasteError::Empty);
    }
    if data_rows.iter().any(|row| row.len() != columns.len()) {
        return Err(GridPasteError::UnevenRows);
    }
    if anchor.row.saturating_add(data_rows.len()) > page.rows.len()
        || columns.iter().any(|column| *column >= page.columns.len())
    {
        return Err(GridPasteError::OutOfBounds);
    }

    let mut assignments = Vec::with_capacity(data_rows.len() * columns.len());
    for (row_offset, fields) in data_rows.iter().enumerate() {
        for (field, column_index) in fields.iter().zip(&columns) {
            let value = page.columns[*column_index]
                .parse_input(field, field == "\\N")
                .map_err(|error| GridPasteError::InvalidCell {
                    row: anchor.row + row_offset,
                    column: *column_index,
                    error,
                })?;
            assignments.push((
                GridCell {
                    row: anchor.row + row_offset,
                    column: *column_index,
                },
                value,
            ));
        }
    }
    Ok(assignments)
}

pub(super) fn grid_paste_error_message(error: GridPasteError, language: UiLanguage) -> String {
    match error {
        GridPasteError::NoTarget => text(
            language,
            "请先选择粘贴起始单元格。",
            "Select a destination cell before pasting.",
        )
        .to_string(),
        GridPasteError::Empty => {
            text(language, "没有可粘贴的数据。", "There is no data to paste.").to_string()
        }
        GridPasteError::InvalidDelimited(error) => format!(
            "{} {error}",
            text(
                language,
                "无法解析剪贴板中的 CSV/TSV 数据：",
                "Could not parse the clipboard CSV/TSV data:"
            )
        ),
        GridPasteError::UnevenRows => text(
            language,
            "剪贴板中的行具有不同的列数。",
            "Clipboard rows contain different numbers of columns.",
        )
        .to_string(),
        GridPasteError::OutOfBounds => text(
            language,
            "粘贴区域超出当前页的行或列范围。",
            "The paste range extends beyond the current page.",
        )
        .to_string(),
        GridPasteError::InvalidCell { row, column, error } => format!(
            "{} {}，{} {}：{}",
            text(language, "行", "Row"),
            row + 1,
            text(language, "列", "column"),
            column + 1,
            cell_input_error_message(error, language)
        ),
    }
}

pub(super) fn cell_input_error_message(error: GridCellInputError, language: UiLanguage) -> String {
    match error {
        GridCellInputError::NullNotAllowed => text(
            language,
            "此列不允许 NULL。请输入一个值。",
            "This column does not allow NULL. Enter a value.",
        ),
        GridCellInputError::ExpectedBoolean => text(
            language,
            "请输入 true、false、1 或 0。",
            "Enter true, false, 1, or 0.",
        ),
        GridCellInputError::ExpectedNumber => {
            text(language, "请输入有效数字。", "Enter a valid number.")
        }
        GridCellInputError::ExpectedInteger => {
            text(language, "请输入整数。", "Enter a whole number.")
        }
        GridCellInputError::ExpectedDate => text(
            language,
            "请输入 YYYY-MM-DD 日期。",
            "Enter a YYYY-MM-DD date.",
        ),
        GridCellInputError::ExpectedTime => text(
            language,
            "请输入 HH:MM:SS 时间，可包含小数秒或时区。",
            "Enter an HH:MM:SS time, optionally with fractional seconds or an offset.",
        ),
        GridCellInputError::ExpectedDateTime => text(
            language,
            "请输入 ISO 8601 日期时间。",
            "Enter an ISO 8601 date and time.",
        ),
        GridCellInputError::ExpectedEnum => text(
            language,
            "请选择允许的枚举值。",
            "Choose one of the allowed enum values.",
        ),
        GridCellInputError::EnumValuesUnavailable => text(
            language,
            "无法加载此枚举的可选值。",
            "The allowed enum values are unavailable.",
        ),
        GridCellInputError::InvalidJson => text(language, "请输入有效 JSON。", "Enter valid JSON."),
    }
    .to_string()
}

pub(super) fn grid_error_message(error: GridSessionError, language: UiLanguage) -> String {
    match error {
        GridSessionError::Loading => text(language, "表格仍在加载。", "The grid is still loading."),
        GridSessionError::Saving => text(language, "更改正在保存。", "Changes are being saved."),
        GridSessionError::Unavailable => text(
            language,
            "连接会话已更改。请重新打开表格。",
            "The connection session changed. Reopen the grid.",
        ),
        GridSessionError::PendingChanges => text(
            language,
            "请先保存或放弃更改。",
            "Save or discard changes first.",
        ),
        GridSessionError::NoChanges => text(
            language,
            "没有要保存的更改。",
            "There are no changes to save.",
        ),
        GridSessionError::AwaitingData => {
            text(language, "表数据尚未就绪。", "Table data is not ready.")
        }
        GridSessionError::ReadOnlyEngine(_) => text(
            language,
            "此数据库引擎的表格为只读。",
            "Data grids are read-only for this database engine.",
        ),
        GridSessionError::MissingPrimaryKey => text(
            language,
            "此表没有主键，不能安全编辑。",
            "This table has no primary key and cannot be edited safely.",
        ),
        GridSessionError::CompositePrimaryKey => text(
            language,
            "暂不支持编辑复合主键表。",
            "Editing tables with composite primary keys is not supported yet.",
        ),
        GridSessionError::DeletedRow(_) => text(
            language,
            "已删除的行不能编辑。",
            "A deleted row cannot be edited.",
        ),
        GridSessionError::MissingRowIdentity(_) => text(
            language,
            "无法确定原始行的主键值。",
            "The original row primary-key value is unavailable.",
        ),
        GridSessionError::InvalidPage
        | GridSessionError::InvalidPageSize
        | GridSessionError::InvalidPageShape { .. }
        | GridSessionError::RowOutOfBounds(_)
        | GridSessionError::ColumnOutOfBounds(_)
        | GridSessionError::UnknownSortColumn(_)
        | GridSessionError::DuplicateSortColumn(_)
        | GridSessionError::DraftNotFound(_) => text(
            language,
            "表格状态已更改。请刷新后重试。",
            "The grid state changed. Refresh and try again.",
        ),
    }
    .to_string()
}

pub(super) fn editability_label(editability: GridEditability<'_>, language: UiLanguage) -> String {
    match editability {
        GridEditability::Editable {
            primary_key_column, ..
        } => format!(
            "{}: {primary_key_column}",
            text(language, "可编辑 · 主键", "Editable · Primary key")
        ),
        GridEditability::ReadOnlyEngine(db_type) => format!(
            "{}: {db_type:?}",
            text(language, "只读引擎", "Read-only engine")
        ),
        GridEditability::MissingPrimaryKey => text(
            language,
            "只读 · 表中没有主键",
            "Read-only · Table has no primary key",
        )
        .to_string(),
        GridEditability::CompositePrimaryKey => text(
            language,
            "只读 · 暂不支持复合主键",
            "Read-only · Composite primary keys are not supported yet",
        )
        .to_string(),
        GridEditability::AwaitingData => {
            text(language, "正在确定编辑能力", "Checking editability").to_string()
        }
    }
}

pub(super) fn change_summary_message(
    summary: crate::application::GridChangeSummary,
    language: UiLanguage,
) -> String {
    let mut parts = Vec::new();
    match language {
        UiLanguage::Chinese => {
            if summary.updated_cells > 0 {
                parts.push(format!("{} 个单元格", summary.updated_cells));
            }
            if summary.inserted_rows > 0 {
                parts.push(format!("{} 个新增行", summary.inserted_rows));
            }
            if summary.deleted_rows > 0 {
                parts.push(format!("{} 个删除行", summary.deleted_rows));
            }
            format!("未保存：{}", parts.join(" · "))
        }
        UiLanguage::English => {
            if summary.updated_cells > 0 {
                parts.push(format!("{} cells", summary.updated_cells));
            }
            if summary.inserted_rows > 0 {
                parts.push(format!("{} inserted rows", summary.inserted_rows));
            }
            if summary.deleted_rows > 0 {
                parts.push(format!("{} deleted rows", summary.deleted_rows));
            }
            format!("Unsaved: {}", parts.join(" · "))
        }
    }
}

pub(super) fn save_outcome_message(outcome: GridSaveOutcome, language: UiLanguage) -> String {
    match language {
        UiLanguage::Chinese => format!(
            "已保存 {} 项更改 · 执行 {} 条语句 · 影响 {} 行",
            outcome.changes_applied, outcome.statements_executed, outcome.affected_rows
        ),
        UiLanguage::English => format!(
            "Saved {} changes · {} statements · {} affected rows",
            outcome.changes_applied, outcome.statements_executed, outcome.affected_rows
        ),
    }
}

pub(super) fn clamped_active_cell(
    page: Option<&GridPage>,
    current: Option<GridCell>,
) -> Option<GridCell> {
    let page = page?;
    if page.rows.is_empty() || page.columns.is_empty() {
        return None;
    }
    let current = current.unwrap_or(GridCell { row: 0, column: 0 });
    Some(GridCell {
        row: current.row.min(page.rows.len() - 1),
        column: current.column.min(page.columns.len() - 1),
    })
}

pub(super) fn moved_grid_cell(
    page: &GridPage,
    current: GridCell,
    row_delta: isize,
    column_delta: isize,
) -> GridCell {
    let move_axis = |position: usize, delta: isize, upper_bound: usize| {
        if delta < 0 {
            position.saturating_sub(delta.unsigned_abs())
        } else {
            position
                .saturating_add(delta as usize)
                .min(upper_bound.saturating_sub(1))
        }
    };
    GridCell {
        row: move_axis(current.row, row_delta, page.rows.len()),
        column: move_axis(current.column, column_delta, page.columns.len()),
    }
}

pub(super) fn next_sort(column: &str, current: &[GridSort]) -> Vec<GridSort> {
    let direction = match current.first() {
        Some(sort) if sort.column == column && sort.direction == GridSortDirection::Ascending => {
            Some(GridSortDirection::Descending)
        }
        Some(sort) if sort.column == column && sort.direction == GridSortDirection::Descending => {
            None
        }
        _ => Some(GridSortDirection::Ascending),
    };
    direction
        .map(|direction| {
            vec![GridSort {
                column: column.to_string(),
                direction,
            }]
        })
        .unwrap_or_default()
}

pub(super) fn can_advance(page: &GridPage, page_number: u32, page_size: u32) -> bool {
    match page.total_rows {
        Some(total) => u64::from(page_number) * u64::from(page_size) < total,
        None => page.rows.len() == page_size as usize,
    }
}

pub(super) fn page_summary(
    language: crate::platform::UiLanguage,
    page: u32,
    rows: usize,
    total_rows: Option<u64>,
) -> String {
    match (language, total_rows) {
        (crate::platform::UiLanguage::Chinese, Some(total)) => {
            format!("第 {page} 页 · {rows}/{total} 行")
        }
        (crate::platform::UiLanguage::Chinese, None) => format!("第 {page} 页 · {rows} 行"),
        (_, Some(total)) => format!("Page {page} · {rows}/{total} rows"),
        (_, None) => format!("Page {page} · {rows} rows"),
    }
}

pub(super) fn centered_grid_state(
    status: GridSessionStatus<'_>,
    language: crate::platform::UiLanguage,
) -> AnyElement {
    let (title, detail, color) = match status {
        GridSessionStatus::Idle | GridSessionStatus::Loading => (
            text(language, "正在加载表数据…", "Loading table data…"),
            text(
                language,
                "正在读取列和当前页。",
                "Reading columns and the current page.",
            ),
            Color::Muted,
        ),
        GridSessionStatus::Failed { error } => (
            text(language, "无法加载表数据", "Could not load table data"),
            error,
            Color::Error,
        ),
        GridSessionStatus::Unavailable { reason } => (
            text(language, "表数据已失效", "Table data is no longer live"),
            reason,
            Color::Warning,
        ),
        GridSessionStatus::Saving => (
            text(language, "正在保存更改…", "Saving changes…"),
            text(
                language,
                "保存完成后将重新加载数据。",
                "Data will reload after saving.",
            ),
            Color::Muted,
        ),
        GridSessionStatus::SaveFailed { error } => (
            text(language, "无法保存更改", "Could not save changes"),
            error,
            Color::Error,
        ),
        GridSessionStatus::Ready => (
            text(language, "当前页没有数据", "No rows on this page"),
            text(
                language,
                "可以返回上一页或刷新数据。",
                "Go back or refresh the data.",
            ),
            Color::Muted,
        ),
    };
    v_flex()
        .size_full()
        .justify_center()
        .items_center()
        .gap_1()
        .p_6()
        .text_center()
        .child(Label::new(title).size(LabelSize::Small).color(color))
        .child(
            Label::new(detail.to_string())
                .size(LabelSize::XSmall)
                .color(color)
                .line_clamp(4),
        )
        .into_any_element()
}
