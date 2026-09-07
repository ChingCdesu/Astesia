use super::*;

#[derive(Clone, Copy)]
enum CellMenuAction {
    Copy,
    CopyWithHeaders,
    Paste,
    DeleteRows,
}

impl DataGridItem {
    pub(super) fn open_cell_menu(
        &mut self,
        cell: GridCell,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.commit_cell_edit(window, cx) {
            return;
        }
        let selected = self.state.row_selected(cell.row)
            || self
                .state
                .cell_selection()
                .is_some_and(|selection| selection.contains(cell));
        if !selected && self.state.select_cell(cell, false).is_err() {
            return;
        }
        self.active_cell = Some(cell);
        let language = self.settings.read(cx).language();
        let editable = matches!(self.state.editability(), GridEditability::Editable { .. });
        let blocked = matches!(
            self.state.status(),
            GridSessionStatus::Saving
                | GridSessionStatus::Loading
                | GridSessionStatus::Unavailable { .. }
        ) || self.transaction_busy
            || self
                .transaction
                .as_ref()
                .is_some_and(|transaction| transaction.is_closed())
            || self.save_recovery_sql.is_some();
        use CellMenuAction::*;
        let owner = cx.entity().downgrade();
        let menu = ContextMenu::build(window, cx, |mut menu, _, _| {
            for (label, action, disabled) in [
                (text(language, "复制", "Copy"), Copy, false),
                (
                    text(language, "复制含表头", "Copy + Headers"),
                    CopyWithHeaders,
                    false,
                ),
                (text(language, "粘贴", "Paste"), Paste, blocked),
                (
                    text(language, "删除所选行", "Delete Selected"),
                    DeleteRows,
                    blocked || self.state.selected_row_count() == 0,
                ),
            ] {
                if matches!(action, Paste | DeleteRows) && !editable {
                    continue;
                }
                if matches!(action, DeleteRows) {
                    menu = menu.separator();
                }
                let owner = owner.clone();
                menu = menu.item(ContextMenuEntry::new(label).disabled(disabled).handler(
                    move |window, cx| {
                        owner
                            .update(cx, |item, cx| match action {
                                Copy => item.copy_selection(false, cx),
                                CopyWithHeaders => item.copy_selection(true, cx),
                                Paste => item.paste_selection(cx),
                                DeleteRows => item.delete_selected_rows(window, cx),
                            })
                            .ok();
                    },
                ));
            }
            menu
        });
        window.focus(&menu.focus_handle(cx), cx);
        let subscription =
            cx.subscribe_in(&menu, window, |item, menu, _: &DismissEvent, window, cx| {
                if menu.focus_handle(cx).contains_focused(window, cx) {
                    window.focus(&item.focus_handle, cx);
                }
                item.context_menu = None;
                cx.notify();
            });
        self.context_menu = Some((menu, position, subscription));
        cx.notify();
    }
}
