use super::*;
use gpui_kit::component::{
    table::{Column, ColumnSort, DataTable, TableDelegate, TableState},
    Sizable,
};

pub(super) struct GridTableDelegate {
    owner: WeakEntity<DataGridItem>,
    columns: Vec<Column>,
    rows: usize,
    language: UiLanguage,
}

impl GridTableDelegate {
    pub(super) fn new(owner: WeakEntity<DataGridItem>) -> Self {
        Self {
            owner,
            columns: Vec::new(),
            rows: 0,
            language: UiLanguage::English,
        }
    }
}

impl TableDelegate for GridTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }
    fn rows_count(&self, _: &App) -> usize {
        self.rows
    }
    fn column(&self, index: usize, _: &App) -> Column {
        self.columns[index].clone()
    }

    fn render_th(
        &mut self,
        index: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let owner = self.owner.clone();
        div()
            .id(("grid-header", index))
            .size_full()
            .px_2()
            .child(
                Label::new(self.columns[index].name.clone())
                    .buffer_font(cx)
                    .size(LabelSize::XSmall)
                    .weight(FontWeight::SEMIBOLD),
            )
            .on_click(move |_, window, cx| {
                if index > 0 {
                    owner
                        .update(cx, |item, cx| item.sort_column(index - 1, window, cx))
                        .ok();
                }
                cx.stop_propagation();
            })
    }

    fn perform_sort(
        &mut self,
        index: usize,
        _: ColumnSort,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        if index > 0 {
            self.owner
                .update(cx, |item, cx| item.sort_column(index - 1, window, cx))
                .ok();
        }
    }

    fn render_td(
        &mut self,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        self.owner
            .update(cx, |item, cx| {
                #[cfg(test)]
                item.rendered_rows.borrow_mut().push(row);
                let existing_rows = item.state.page().map_or(0, |page| page.rows.len());
                if column == 0 {
                    item.render_row_number(row, existing_rows, cx)
                } else if row < existing_rows {
                    item.render_existing_cell(row, column - 1, window, cx)
                } else {
                    item.render_draft_cell(row - existing_rows, column - 1, cx)
                }
            })
            .unwrap_or_else(|_| div().into_any_element())
    }

    fn render_tr(
        &mut self,
        row: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let (background, deleted) = self
            .owner
            .read_with(cx, |item, cx| {
                let deleted = item.state.is_row_deleted(row);
                let background =
                    if deleted || row >= item.state.page().map_or(0, |page| page.rows.len()) {
                        cx.theme().status().warning_background
                    } else if item.state.row_selected(row) {
                        cx.theme().colors().ghost_element_selected
                    } else {
                        cx.theme().colors().editor_background
                    };
                (background, deleted)
            })
            .unwrap_or((cx.theme().colors().editor_background, false));
        div()
            .id(("grid-row", row))
            .bg(background)
            .when(deleted, |row| row.opacity(0.72))
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        grid_view::grid_empty_placeholder(self.language).size_full()
    }

    fn render_last_empty_col(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div().w_0()
    }
}

impl DataGridItem {
    pub(super) fn render_grid(&self, page: &GridPage, cx: &mut Context<Self>) -> AnyElement {
        let locked =
            self.has_local_changes() || self.transaction_busy || self.save_recovery_sql.is_some();
        let mut columns = vec![Column::new("__row_number", "#")
            .width(ROW_NUMBER_WIDTH)
            .resizable(false)
            .movable(false)
            .paddings(px(0.0))];
        columns.extend(page.columns.iter().enumerate().map(|(index, column)| {
            let mut result = Column::new(format!("column-{index}"), column.name.clone())
                .width(self.column_width(index))
                .movable(false)
                .paddings(px(0.0));
            result.min_width = px(MIN_COLUMN_WIDTH);
            result.max_width = px(MAX_COLUMN_WIDTH);
            if !locked {
                result.sort = Some(
                    match self
                        .state
                        .query()
                        .sort
                        .iter()
                        .find(|sort| sort.column == column.name)
                        .map(|sort| sort.direction)
                    {
                        Some(GridSortDirection::Ascending) => ColumnSort::Ascending,
                        Some(GridSortDirection::Descending) => ColumnSort::Descending,
                        None => ColumnSort::Default,
                    },
                );
            }
            result
        }));
        self.data_table.update(cx, |table, cx| {
            let current = &table.delegate().columns;
            let changed = current.len() != columns.len()
                || current
                    .iter()
                    .zip(&columns)
                    .any(|(a, b)| a.name != b.name || a.width != b.width || a.sort != b.sort);
            let delegate = table.delegate_mut();
            delegate.columns = columns;
            delegate.rows = page.rows.len() + self.state.drafts().len();
            delegate.language = self.settings.read(cx).language();
            if changed {
                table.refresh(cx);
            }
            cx.notify();
        });
        DataTable::new(&self.data_table)
            .with_size(grid_view::GRID_ROW_HEIGHT)
            .bordered(false)
            .into_any_element()
    }

    fn render_row_number(
        &self,
        row: usize,
        existing_rows: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let language = self.settings.read(cx).language();
        let cell = h_flex()
            .id(("grid-row-number", row))
            .size_full()
            .px_2()
            .border_r_1()
            .border_color(cx.theme().colors().border);
        if row >= existing_rows {
            let Some(draft) = self.state.drafts().get(row - existing_rows) else {
                return cell.into_any_element();
            };
            let id = draft.id;
            cell.child(
                IconButton::new(format!("remove-draft-{id}"), IconName::Close)
                    .size(ButtonSize::Compact)
                    .aria_label(text(language, "移除新行", "Remove new row"))
                    .on_click(cx.listener(move |item, _, window, cx| {
                        item.remove_draft_row(id, window, cx)
                    })),
            )
            .into_any_element()
        } else {
            let label = u64::from(self.state.query().page.saturating_sub(1))
                * u64::from(self.state.query().page_size)
                + row as u64
                + 1;
            let deleted = self.state.is_row_deleted(row);
            cell.role(gpui_kit::Role::RowHeader)
                .aria_label(format!(
                    "{} {label}{}",
                    text(language, "行", "Row"),
                    if deleted {
                        text(language, "，已标记为待删除", ", marked for deletion")
                    } else {
                        ""
                    }
                ))
                .aria_selected(self.state.row_selected(row))
                .child(if deleted {
                    Icon::new(IconName::Trash)
                        .size(IconSize::XSmall)
                        .color(Color::Warning)
                        .into_any_element()
                } else {
                    Label::new(label.to_string())
                        .buffer_font(cx)
                        .size(LabelSize::XSmall)
                        .into_any_element()
                })
                .when(!deleted, |cell| {
                    cell.cursor_pointer()
                        .on_click(cx.listener(move |item, event, window, cx| {
                            item.select_row(row, event, window, cx)
                        }))
                })
                .into_any_element()
        }
    }
}
