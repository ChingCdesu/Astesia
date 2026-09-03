use std::path::PathBuf;

use gpui::{ClickEvent, Context, PromptButton, PromptLevel, Window};

use crate::application::{
    CsvOptions, ExportFormat, ExportSource, GridSession, GridSortDirection, JsonLayout,
    JsonOptions, XlsxOptions,
};
use crate::db::{SqlDialect, SqlScript};

use super::{text, DataGridItem, GridExportFormat, GridNotice};

impl DataGridItem {
    pub(super) fn export_data(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.export_in_progress || self.state.page().is_none() {
            return;
        }
        let language = self.settings.read(cx).language();
        let prompt = window.prompt(
            PromptLevel::Info,
            text(language, "选择导出格式", "Choose export format"),
            Some(text(
                language,
                "所有格式都会保留当前选择的列。",
                "Every format preserves the currently selected columns.",
            )),
            &[
                PromptButton::ok("CSV"),
                PromptButton::new("JSON"),
                PromptButton::new("XLSX"),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |item, cx| {
            let format = match prompt.await.ok() {
                Some(0) => Some(GridExportFormat::Csv),
                Some(1) => Some(GridExportFormat::Json),
                Some(2) => Some(GridExportFormat::Xlsx),
                _ => None,
            };
            if let Some(format) = format {
                item.update_in(cx, |item, window, cx| {
                    item.choose_export_scope(format, window, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    fn choose_export_scope(
        &mut self,
        format: GridExportFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let language = self.settings.read(cx).language();
        let prompt = window.prompt(
            PromptLevel::Info,
            text(language, "选择导出范围", "Choose export scope"),
            Some(text(
                language,
                "当前范围会导出页面或选区；全部范围会应用当前筛选和排序。",
                "Current exports the page or selection; All applies the current filter and sort.",
            )),
            &[
                PromptButton::ok(text(
                    language,
                    "当前页面或选区",
                    "Current Page or Selection",
                )),
                PromptButton::new(text(language, "全部匹配行", "All Matching Rows")),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |item, cx| {
            let all_rows = match prompt.await.ok() {
                Some(0) => Some(false),
                Some(1) => Some(true),
                _ => None,
            };
            if let Some(all_rows) = all_rows {
                item.update_in(cx, |item, window, cx| {
                    item.choose_export_path(format, all_rows, window, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    fn choose_export_path(
        &mut self,
        format: GridExportFormat,
        all_rows: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let default_name = format!("{}.{}", self.state.table().name(), format.extension());
        let prompt = cx.prompt_for_new_path(&PathBuf::default(), Some(&default_name));
        cx.spawn_in(window, async move |item, cx| {
            let response = prompt.await;
            item.update_in(cx, |item, _, cx| match response {
                Ok(Ok(Some(path))) => item.start_export(path, format, all_rows, cx),
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    item.operation_notice = Some(GridNotice::Error(format!(
                        "{}: {error}",
                        text(
                            item.settings.read(cx).language(),
                            "无法打开导出文件选择器",
                            "Could not open the export file picker"
                        )
                    )));
                    cx.notify();
                }
                Err(error) => {
                    item.operation_notice = Some(GridNotice::Error(format!(
                        "{}: {error}",
                        text(
                            item.settings.read(cx).language(),
                            "导出文件选择器意外结束",
                            "The export file picker ended unexpectedly"
                        )
                    )));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn start_export(
        &mut self,
        path: PathBuf,
        format: GridExportFormat,
        all_rows: bool,
        cx: &mut Context<Self>,
    ) {
        let source = if all_rows {
            match grid_export_sql(&self.state) {
                Ok(sql) => ExportSource::Sql { sql },
                Err(error) => {
                    self.operation_notice = Some(GridNotice::Error(error));
                    cx.notify();
                    return;
                }
            }
        } else {
            let Some((columns, rows)) = self.state.export_rows() else {
                return;
            };
            ExportSource::Rows { columns, rows }
        };
        let export_format = format.export_format(self.state.table().name());
        if self.export_in_progress {
            return;
        }
        self.export_in_progress = true;
        self.operation_notice = None;
        cx.notify();
        let application = self.application.clone();
        let target = self.state.target().clone();
        let path_label = path.display().to_string();
        let export = gpui_tokio::Tokio::spawn(cx, async move {
            application
                .exports()
                .start_export(target, source, export_format, path_label.clone())
                .await
                .map(|task_id| (task_id, path_label))
        });
        cx.spawn(async move |item, cx| {
            let result = match export.await {
                Ok(result) => result,
                Err(error) => Err(error.to_string()),
            };
            item.update(cx, |item, cx| {
                item.export_in_progress = false;
                item.operation_notice = Some(match result {
                    Ok((task_id, path)) => GridNotice::Success(format!(
                        "{} {task_id}: {path}",
                        text(
                            item.settings.read(cx).language(),
                            "导出任务已启动",
                            "Export task started"
                        )
                    )),
                    Err(error) => GridNotice::Error(error),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl GridExportFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
            Self::Xlsx => "xlsx",
        }
    }

    fn export_format(self, sheet_name: &str) -> ExportFormat {
        match self {
            Self::Csv => ExportFormat::Csv(CsvOptions {
                delimiter: ",".to_string(),
                include_header: true,
                quote_all: false,
                null_value: "\\N".to_string(),
                crlf: false,
                bom: false,
            }),
            Self::Json => ExportFormat::Json(JsonOptions {
                layout: JsonLayout::Objects,
                pretty: true,
            }),
            Self::Xlsx => ExportFormat::Xlsx(XlsxOptions {
                include_header: true,
                sheet_name: sheet_name.to_string(),
            }),
        }
    }
}

fn grid_export_sql(state: &GridSession) -> Result<String, String> {
    let (columns, _) = state
        .export_rows()
        .ok_or_else(|| "No grid rows are available to determine export columns".to_string())?;
    if columns.is_empty() {
        return Err("At least one export column is required".to_string());
    }
    let dialect = SqlDialect::new(state.target().db_type);
    let columns = columns
        .iter()
        .map(|column| dialect.quote_identifier(column))
        .collect::<Result<Vec<_>, _>>()?;
    let mut sql = format!(
        "SELECT {} FROM {}",
        columns.join(", "),
        dialect.quote_table_ref(state.table())?
    );
    if let Some(filter) = state
        .query()
        .filter
        .as_deref()
        .map(str::trim)
        .filter(|filter| !filter.is_empty())
    {
        sql.push_str(" WHERE ");
        sql.push_str(filter);
    }
    if !state.query().sort.is_empty() {
        let order = state
            .query()
            .sort
            .iter()
            .map(|sort| {
                let direction = match sort.direction {
                    GridSortDirection::Ascending => "ASC",
                    GridSortDirection::Descending => "DESC",
                };
                Ok(format!(
                    "{} {direction}",
                    dialect.quote_identifier(&sort.column)?
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        sql.push_str(" ORDER BY ");
        sql.push_str(&order.join(", "));
    }
    let statements = SqlScript::parse(state.target().db_type, &sql)
        .map_err(|error| format!("Could not build export query: {error}"))?
        .into_statements();
    if statements.len() != 1 {
        return Err("Grid export must produce exactly one read statement".to_string());
    }
    Ok(statements.into_iter().next().expect("one export statement"))
}
