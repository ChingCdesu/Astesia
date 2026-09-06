use super::*;
use crate::db::TransactionIsolation;
use crate::ui::components::{ContextMenuEntry, IconPosition};

impl DataGridItem {
    pub(super) fn transaction_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let manual = self.manual_transaction;
        let isolation = self.transaction_isolation;
        let engine = self.state.target().db_type;
        let language = self.settings.read(cx).language();
        let locked = self.transaction_busy
            || self.has_unsaved_changes()
            || matches!(
                self.state.status(),
                GridSessionStatus::Loading
                    | GridSessionStatus::Saving
                    | GridSessionStatus::Unavailable { .. }
            );
        let owner = cx.entity().downgrade();
        Button::new(
            "grid-transaction-mode",
            if manual { "Tx: Manual" } else { "Tx: Auto" },
        )
        .size(ButtonSize::Compact)
        .end_icon(Icon::new(IconName::ChevronDown).size(IconSize::XSmall))
        .disabled(self.transaction_busy)
        .popup_menu(move |menu, _, _| {
            let mut menu = menu.header(text(language, "事务模式", "Transaction Mode"));
            for mode in [false, true] {
                let owner = owner.clone();
                menu = menu.item(
                    ContextMenuEntry::new(if mode { "Manual" } else { "Auto" })
                        .toggleable(IconPosition::Start, manual == mode)
                        .disabled(locked)
                        .handler(move |_, cx| {
                            owner
                                .update(cx, |item, cx| {
                                    item.change_transaction_configuration(mode, isolation, cx)
                                })
                                .ok();
                        }),
                );
            }
            menu = menu
                .separator()
                .header(text(language, "事务隔离级别", "Transaction Isolation"));
            for level in engine.transaction_isolations() {
                let level = *level;
                let owner = owner.clone();
                menu = menu.item(
                    ContextMenuEntry::new(isolation_label(level))
                        .toggleable(IconPosition::Start, isolation == level)
                        .disabled(locked)
                        .handler(move |_, cx| {
                            owner
                                .update(cx, |item, cx| {
                                    item.change_transaction_configuration(manual, level, cx)
                                })
                                .ok();
                        }),
                );
            }
            menu.separator().label(if manual {
                text(
                    language,
                    "保存写入事务；提交后才持久化",
                    "Save applies changes; Commit makes them durable",
                )
            } else {
                text(
                    language,
                    "保存的更改自动提交",
                    "Saved changes are automatically committed",
                )
            })
        })
    }

    fn change_transaction_configuration(
        &mut self,
        manual: bool,
        isolation: TransactionIsolation,
        cx: &mut Context<Self>,
    ) {
        if self.save_recovery_sql.is_some()
            || self.transaction_busy
            || self.has_unsaved_changes()
            || !self
                .state
                .target()
                .db_type
                .transaction_isolations()
                .contains(&isolation)
            || matches!(
                self.state.status(),
                GridSessionStatus::Loading
                    | GridSessionStatus::Saving
                    | GridSessionStatus::Unavailable { .. }
            )
        {
            return;
        }
        if !manual && !self.manual_transaction {
            self.transaction_isolation = isolation;
            cx.notify();
            return;
        }
        self.transaction_busy = true;
        let old = self.transaction.take();
        let service = self.application.grids().clone();
        let target = self.state.target().clone();
        let start = crate::ui::runtime::spawn(cx, async move {
            if let Some(old) = old {
                if !old.is_closed() {
                    old.finish(false).await?;
                }
            }
            if manual {
                service.begin_transaction(target, isolation).await.map(Some)
            } else {
                Ok(None)
            }
        });
        cx.spawn(async move |item, cx| {
            let result = start
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result);
            item.update(cx, |item, cx| {
                item.transaction_busy = false;
                match result {
                    Ok(transaction) => {
                        item.transaction = transaction;
                        item.manual_transaction = manual;
                        item.transaction_isolation = isolation;
                        item.operation_notice = None;
                        item.load(cx);
                    }
                    Err(error) => {
                        item.manual_transaction = false;
                        item.operation_notice = Some(GridNotice::Error(error));
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn commit_transaction(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.has_changes()
            || self
                .editing
                .as_ref()
                .is_some_and(|editing| editing.modified)
        {
            return;
        }
        self.finish_transaction(true, cx);
    }

    pub(super) fn rollback_transaction(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_rollback_transaction(window, cx);
    }

    pub(super) fn confirm_rollback_transaction(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.transaction_busy || window.has_active_prompt() {
            return;
        }
        let language = self.settings.read(cx).language();
        let answer = window.prompt(
            PromptLevel::Warning,
            text(
                language,
                "回滚事务并放弃本页更改？",
                "Roll back the transaction and discard changes in this tab?",
            ),
            Some(text(
                language,
                "事务中的更改与尚未保存的编辑都将丢失。",
                "Changes in the transaction and unsaved edits will be lost.",
            )),
            &[
                PromptButton::ok(text(language, "回滚", "Roll Back")),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |item, cx| {
            if answer.await.ok() == Some(0) {
                item.update(cx, |item, cx| item.finish_transaction(false, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn finish_transaction(&mut self, commit: bool, cx: &mut Context<Self>) {
        if self.transaction_busy
            || matches!(
                self.state.status(),
                GridSessionStatus::Saving | GridSessionStatus::Loading
            )
        {
            return;
        }
        let Some(transaction) = self
            .transaction
            .clone()
            .filter(|transaction| !transaction.is_closed())
        else {
            return;
        };
        self.transaction_busy = true;
        let finish = crate::ui::runtime::spawn(cx, async move { transaction.finish(commit).await });
        cx.spawn(async move |item, cx| {
            let result = finish
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result);
            item.update(cx, |item, cx| {
                item.transaction_busy = false;
                match result {
                    Ok(()) => {
                        item.transaction = None;
                        item.editing = None;
                        if !commit {
                            item.state.discard_changes();
                        }
                        item.change_transaction_configuration(true, item.transaction_isolation, cx);
                    }
                    Err(error) => {
                        item.operation_notice = Some(GridNotice::Error(error));
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn copy_transaction_recovery(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(transaction) = self.transaction.as_ref() {
            cx.write_to_clipboard(ClipboardItem::new_string(transaction.recovery_sql()));
        } else if let Some(sql) = self.save_recovery_sql.as_ref() {
            cx.write_to_clipboard(ClipboardItem::new_string(sql.clone()));
        }
    }
}

fn isolation_label(isolation: TransactionIsolation) -> &'static str {
    match isolation {
        TransactionIsolation::DatabaseDefault => "Database Default",
        TransactionIsolation::ReadCommitted => "Read Committed",
        TransactionIsolation::RepeatableRead => "Repeatable Read",
        TransactionIsolation::Serializable => "Serializable",
    }
}
