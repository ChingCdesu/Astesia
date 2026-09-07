use super::*;

pub(super) const GRID_ROW_HEIGHT: Pixels = px(28.0);

impl DataGridItem {
    pub(super) fn render_existing_cell(
        &self,
        row_index: usize,
        column_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let item = self;
        let Some(page) = self.state.page() else {
            return div().into_any_element();
        };
        let colors = cx.theme().colors();
        let status = cx.theme().status();
        let language = self.settings.read(cx).language();
        let grid_focused = self.focus_handle.is_focused(window);
        let deleted = self.state.is_row_deleted(row_index);
        let displayed_row = u64::from(self.state.query().page.saturating_sub(1))
            * u64::from(self.state.query().page_size)
            + row_index as u64
            + 1;
        let cell = GridCell {
            row: row_index,
            column: column_index,
        };
        let active = item.active_cell == Some(cell);
        let dirty = item.state.is_cell_dirty(cell);
        let selected = item
            .state
            .cell_selection()
            .is_some_and(|selection| selection.contains(cell));
        let displayed = item
            .state
            .cell_value(cell)
            .map(display_value)
            .unwrap_or_default();
        let mut aria_label = format!(
            "{}, {} {displayed_row}, {displayed}",
            page.columns[column_index].name,
            text(language, "行", "row")
        );
        if dirty {
            aria_label.push_str(text(language, "，未保存的更改", ", unsaved change"));
        }
        if deleted {
            aria_label.push_str(text(language, "，已标记为待删除", ", marked for deletion"));
        }
        let editor = item
            .editing
            .as_ref()
            .filter(|editing| editing.target == CellEditorTarget::Existing(cell));
        let editing_error = editor.and_then(|editing| editing.error.clone());
        if let Some(error) = &editing_error {
            aria_label.push_str(", ");
            aria_label.push_str(error);
        }
        let cell_content: AnyElement = if let Some(editing) = editor {
            if editing.expanded {
                h_flex()
                    .size_full()
                    .min_h(px(24.0))
                    .px_2()
                    .border_1()
                    .border_color(colors.border_focused)
                    .bg(colors.editor_background)
                    .child(
                        Label::new(text(language, "正在下方编辑…", "Editing below…"))
                            .size(LabelSize::XSmall)
                            .color(Color::Accent),
                    )
                    .into_any_element()
            } else {
                inline_grid_cell(
                    &editing.editor,
                    editing.null_requested,
                    editing_error.is_some(),
                    cx,
                )
                .into_any_element()
            }
        } else {
            div()
                .px_2()
                .py_1()
                .child(
                    Label::new(displayed)
                        .buffer_font(cx)
                        .size(LabelSize::XSmall)
                        .truncate(),
                )
                .into_any_element()
        };
        div()
            .id(format!("data-grid-cell-{row_index}-{column_index}"))
            .debug_selector(move || format!("grid-cell-{row_index}-{column_index}"))
            .relative()
            .role(gpui_kit::Role::GridCell)
            .aria_label(aria_label)
            .aria_selected(selected)
            .when(active, |element| element.aria_active_descendant())
            .w_full()
            .h_full()
            .overflow_hidden()
            .flex_none()
            .border_r_1()
            .border_color(colors.border)
            .when(!deleted, |element| element.cursor_pointer())
            .when(dirty, |element| element.bg(status.warning_background))
            .when(selected, |element| {
                element.bg(colors.ghost_element_selected)
            })
            .when(active && grid_focused, |element| {
                element.border_color(colors.border_focused)
            })
            .when(dirty, |element| {
                element.child(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .m_1()
                        .child(Indicator::dot().color(Color::Warning)),
                )
            })
            .child(cell_content)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |item, event: &MouseDownEvent, window, cx| {
                    item.open_cell_menu(cell, event.position, window, cx);
                    cx.stop_propagation();
                }),
            )
            .when(!deleted, |element| {
                element.on_click(cx.listener(move |item, event, window, cx| {
                    item.select_cell(cell, event, window, cx);
                }))
            })
            .into_any_element()
    }
    pub(super) fn render_draft_cell(
        &self,
        draft_index: usize,
        column_index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let item = self;
        let Some(page) = self.state.page() else {
            return div().into_any_element();
        };
        let Some(draft) = self.state.drafts().get(draft_index) else {
            return div().into_any_element();
        };
        let column = &page.columns[column_index];
        let draft_id = draft.id;
        let colors = cx.theme().colors();
        let language = self.settings.read(cx).language();
        let target = CellEditorTarget::Draft {
            draft_id,
            column: column_index,
        };
        let value = draft.values.get(column_index).cloned().flatten();
        let displayed = value
            .as_ref()
            .map(display_value)
            .unwrap_or_else(|| "DEFAULT".to_string());
        let editor = item
            .editing
            .as_ref()
            .filter(|editing| editing.target == target);
        let editing_error = editor.and_then(|editing| editing.error.clone());
        let mut aria_label = format!(
            "{}, {} {}, {displayed}",
            column.name,
            text(language, "新行", "new row"),
            draft_index + 1
        );
        if let Some(error) = &editing_error {
            aria_label.push_str(", ");
            aria_label.push_str(error);
        }
        let cell_content: AnyElement = if let Some(editing) = editor {
            if editing.expanded {
                h_flex()
                    .size_full()
                    .min_h(px(24.0))
                    .px_2()
                    .border_1()
                    .border_color(colors.border_focused)
                    .bg(colors.editor_background)
                    .child(
                        Label::new(text(language, "正在下方编辑…", "Editing below…"))
                            .size(LabelSize::XSmall)
                            .color(Color::Accent),
                    )
                    .into_any_element()
            } else {
                inline_grid_cell(
                    &editing.editor,
                    editing.null_requested,
                    editing_error.is_some(),
                    cx,
                )
                .into_any_element()
            }
        } else {
            div()
                .px_2()
                .py_1()
                .child(
                    Label::new(displayed)
                        .buffer_font(cx)
                        .size(LabelSize::XSmall)
                        .truncate(),
                )
                .into_any_element()
        };
        div()
            .id(format!("data-grid-draft-cell-{draft_id}-{column_index}"))
            .role(gpui_kit::Role::GridCell)
            .aria_label(aria_label)
            .w_full()
            .h_full()
            .overflow_hidden()
            .flex_none()
            .border_r_1()
            .border_color(colors.border)
            .cursor_pointer()
            .child(cell_content)
            .on_click(cx.listener(move |item, _, window, cx| {
                item.select_draft_cell(draft_id, column_index, window, cx);
            }))
            .into_any_element()
    }
}

fn inline_grid_cell(
    editor: &Entity<Editor>,
    null_requested: bool,
    invalid: bool,
    cx: &App,
) -> gpui_kit::Div {
    let colors = cx.theme().colors();
    h_flex()
        .key_context("DataGridCellEditor")
        .size_full()
        .min_w_0()
        .px(px(7.0))
        .border_1()
        .border_color(if invalid {
            cx.theme().status().error_border
        } else {
            colors.border_focused
        })
        .bg(colors.editor_background)
        .when(null_requested, |element| {
            element.child(
                Label::new("NULL")
                    .buffer_font(cx)
                    .size(LabelSize::XSmall)
                    .color(Color::Warning),
            )
        })
        .child(
            div()
                .min_w_0()
                .h_full()
                .flex_1()
                .when(null_requested, |element| element.opacity(0.45))
                .child(editor.clone()),
        )
}

pub(super) fn grid_empty_placeholder(language: crate::platform::UiLanguage) -> gpui_kit::Div {
    v_flex()
        .debug_selector(|| "grid-empty".into())
        .flex_1()
        .justify_center()
        .items_center()
        .gap_1()
        .child(
            div().debug_selector(|| "empty-title".into()).child(
                Label::new(text(language, "当前页没有数据", "No rows on this page"))
                    .size(LabelSize::Small),
            ),
        )
        .child(
            Label::new(text(
                language,
                "可以返回上一页或刷新数据。",
                "Go to the previous page or refresh the data.",
            ))
            .size(LabelSize::XSmall)
            .color(Color::Muted),
        )
}

#[cfg(test)]
fn grid_row() -> gpui_kit::Div {
    h_flex().h(GRID_ROW_HEIGHT)
}

#[cfg(test)]
mod inline_cell_tests {
    use super::*;

    struct CellTest {
        editor: Entity<Editor>,
        editing: bool,
        null_requested: bool,
        invalid: bool,
    }

    impl Render for CellTest {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let content = if self.editing {
                inline_grid_cell(&self.editor, self.null_requested, self.invalid, cx)
                    .into_any_element()
            } else {
                div()
                    .px_2()
                    .py_1()
                    .child(Label::new("user.artistCertChanged").size(LabelSize::XSmall))
                    .into_any_element()
            };
            v_flex()
                .w(px(180.0))
                .child(
                    grid_row()
                        .debug_selector(|| "cell-row".into())
                        .child(div().w_full().h_full().overflow_hidden().child(content)),
                )
                .child(grid_row().debug_selector(|| "next-row".into()))
        }
    }

    #[gpui_kit::test]
    fn inline_editing_preserves_row_height_and_contains_input(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Light, cx)
        });
        let (view, cx) = cx.add_window_view(|window, cx| CellTest {
            editor: cx.new(|cx| {
                let mut editor = Editor::inline_single_line("Cell value", px(12.0), window, cx);
                editor.set_text("user.artistCertChanged", window, cx);
                editor
            }),
            editing: false,
            null_requested: false,
            invalid: false,
        });
        for (editing, null_requested, invalid) in [
            (false, false, false),
            (true, false, false),
            (true, true, false),
            (true, false, true),
        ] {
            view.update(cx, |view, cx| {
                view.editing = editing;
                view.null_requested = null_requested;
                view.invalid = invalid;
                cx.notify();
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let row = cx.debug_bounds("cell-row").unwrap();
            assert_eq!(row.size.height, GRID_ROW_HEIGHT);
            assert_eq!(cx.debug_bounds("next-row").unwrap().top(), row.bottom());
            if editing {
                let input = view.read_with(cx, |view, cx| view.editor.read(cx).input_bounds(cx));
                assert!(input.top() >= row.top());
                assert!(
                    input.bottom() <= row.bottom(),
                    "input exceeds row: {input:?} / {row:?}"
                );
            }
        }
    }
}
