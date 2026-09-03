use std::sync::Arc;

use gpui::{DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, Window};
use ui_input::InputField;
use workspace::{DismissDecision, ModalView};
use zed_ui::{prelude::*, ElevationIndex, Modal, ModalFooter, ModalHeader, Section};

use crate::{
    application::{Application, CopyContent, CopyOptions, QueryTarget},
    db::TableRef,
    platform::UiLanguage,
};

use super::localization::text;

#[derive(Clone, Debug)]
pub(super) struct TransferTaskStarted {
    pub(super) task_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CopyFormOperation {
    Idle,
    Starting,
}

pub(super) struct CopyTableForm {
    application: Arc<Application>,
    source: QueryTarget,
    table: TableRef,
    target_connection: Entity<InputField>,
    target_database: Entity<InputField>,
    new_table_name: Entity<InputField>,
    content: CopyContent,
    operation: CopyFormOperation,
    error: Option<String>,
    language: UiLanguage,
}

impl EventEmitter<DismissEvent> for CopyTableForm {}
impl EventEmitter<TransferTaskStarted> for CopyTableForm {}

impl CopyTableForm {
    pub(super) fn new(
        application: Arc<Application>,
        source: QueryTarget,
        target: QueryTarget,
        table: TableRef,
        language: UiLanguage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let target_connection = input(
            window,
            cx,
            text(language, "Connection Profile ID", "Connection Profile ID"),
            text(language, "目标连接", "Target connection"),
        );
        let target_database = input(
            window,
            cx,
            text(language, "数据库名称", "Database name"),
            text(language, "目标数据库", "Target database"),
        );
        let new_table_name = input(
            window,
            cx,
            text(language, "新表名称", "New table name"),
            text(language, "目标表名称", "Target table name"),
        );
        set_text(&target_connection, &target.connection_id, window, cx);
        set_text(&target_database, &target.database, window, cx);
        set_text(
            &new_table_name,
            &format!("{}_copy", table.name()),
            window,
            cx,
        );
        Self {
            application,
            source,
            table,
            target_connection,
            target_database,
            new_table_name,
            content: CopyContent::StructureAndData,
            operation: CopyFormOperation::Idle,
            error: None,
            language,
        }
    }

    fn set_content(&mut self, content: CopyContent, cx: &mut Context<Self>) {
        if self.operation == CopyFormOperation::Idle && self.content != content {
            self.content = content;
            cx.notify();
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.operation != CopyFormOperation::Idle {
            return;
        }
        let target_connection_id = self.target_connection.read(cx).text(cx).trim().to_string();
        let target_database = self.target_database.read(cx).text(cx).trim().to_string();
        let new_table_name = self.new_table_name.read(cx).text(cx).trim().to_string();
        if target_connection_id.is_empty()
            || target_database.is_empty()
            || new_table_name.is_empty()
        {
            self.error = Some(
                text(
                    self.language,
                    "目标连接、数据库和表名均为必填项。",
                    "Target connection, database, and table name are required.",
                )
                .to_string(),
            );
            cx.notify();
            return;
        }
        self.operation = CopyFormOperation::Starting;
        self.error = None;
        cx.notify();
        let application = self.application.clone();
        let source = self.source.clone();
        let source_table = self.table.clone();
        let content = self.content;
        let start = gpui_tokio::Tokio::spawn(cx, async move {
            let snapshot = application
                .connections()
                .snapshot()
                .await
                .map_err(|error| error.to_string())?;
            let target_profile = snapshot
                .profiles
                .into_iter()
                .find(|profile| profile.profile.id == target_connection_id)
                .ok_or_else(|| "Target Connection Profile was not found".to_string())?;
            let generation = target_profile
                .session
                .generation
                .ok_or_else(|| "Target Connection Profile is not connected".to_string())?;
            if target_profile.profile.db_type != source.db_type {
                return Err("Source and target must use the same database engine".to_string());
            }
            let target = QueryTarget {
                connection_id: target_profile.profile.id.clone(),
                connection_name: target_profile.profile.name.clone(),
                database: target_database,
                db_type: target_profile.profile.db_type,
                session_generation: generation,
            };
            application
                .transfers()
                .start_table_copy(
                    source,
                    source_table,
                    target,
                    CopyOptions {
                        content,
                        new_table_name,
                    },
                )
                .await
        });
        cx.spawn_in(window, async move |form, cx| {
            let result = match start.await {
                Ok(result) => result,
                Err(error) => Err(error.to_string()),
            };
            form.update_in(cx, |form, _, cx| match result {
                Ok(task_id) => {
                    form.operation = CopyFormOperation::Idle;
                    cx.emit(TransferTaskStarted { task_id });
                    cx.emit(DismissEvent);
                }
                Err(error) => {
                    form.operation = CopyFormOperation::Idle;
                    form.error = Some(error);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn cancel(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.operation == CopyFormOperation::Idle {
            cx.emit(DismissEvent);
        }
    }
}

impl Render for CopyTableForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.operation == CopyFormOperation::Starting;
        let choices = [
            (
                CopyContent::StructureAndData,
                text(self.language, "结构和数据", "Structure + Data"),
            ),
            (
                CopyContent::Structure,
                text(self.language, "仅结构", "Structure Only"),
            ),
            (
                CopyContent::Data,
                text(self.language, "仅数据", "Data Only"),
            ),
        ];
        div()
            .tab_group()
            .track_focus(&self.focus_handle(cx))
            .elevation_3(cx)
            .occlude()
            .w(rems(42.0))
            .child(
                Modal::new("copy-table-form", None)
                    .header(
                        ModalHeader::new()
                            .headline(text(self.language, "复制表", "Copy Table"))
                            .description(format!(
                                "{} · {} / {}",
                                self.table, self.source.connection_name, self.source.database
                            ))
                            .show_dismiss_button(!busy),
                    )
                    .section(
                        Section::new().child(
                            v_flex()
                                .gap_3()
                                .child(self.target_connection.clone())
                                .child(self.target_database.clone())
                                .child(self.new_table_name.clone())
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(
                                            Label::new(text(
                                                self.language,
                                                "复制内容",
                                                "Copy content",
                                            ))
                                            .size(LabelSize::Small),
                                        )
                                        .child(h_flex().gap_1().children(choices.into_iter().map(
                                            |(content, label)| {
                                                Button::new(
                                                    format!("copy-content-{content:?}"),
                                                    label,
                                                )
                                                .size(ButtonSize::Compact)
                                                .toggle_state(self.content == content)
                                                .disabled(busy)
                                                .on_click(cx.listener(move |form, _, _, cx| {
                                                    form.set_content(content, cx)
                                                }))
                                            },
                                        ))),
                                )
                                .when_some(self.error.clone(), |element, error| {
                                    element.child(
                                        Label::new(error)
                                            .size(LabelSize::XSmall)
                                            .color(Color::Error)
                                            .line_clamp(4),
                                    )
                                }),
                        ),
                    )
                    .footer(
                        ModalFooter::new().end_slot(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new(
                                        "cancel-table-copy",
                                        text(self.language, "取消", "Cancel"),
                                    )
                                    .disabled(busy)
                                    .on_click(cx.listener(Self::cancel)),
                                )
                                .child(
                                    Button::new(
                                        "start-table-copy",
                                        text(self.language, "开始复制", "Start Copy"),
                                    )
                                    .style(ButtonStyle::Filled)
                                    .layer(ElevationIndex::ModalSurface)
                                    .loading(busy)
                                    .disabled(busy)
                                    .on_click(
                                        cx.listener(|form, _, window, cx| form.submit(window, cx)),
                                    ),
                                ),
                        ),
                    ),
            )
    }
}

impl Focusable for CopyTableForm {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.target_connection.read(cx).focus_handle(cx)
    }
}

impl ModalView for CopyTableForm {
    fn fade_out_background(&self) -> bool {
        true
    }

    fn on_before_dismiss(&mut self, _: &mut Window, _: &mut Context<Self>) -> DismissDecision {
        DismissDecision::Dismiss(self.operation == CopyFormOperation::Idle)
    }
}

fn input(
    window: &mut Window,
    cx: &mut Context<CopyTableForm>,
    placeholder: &str,
    label: &str,
) -> Entity<InputField> {
    cx.new(|cx| InputField::new(window, cx, placeholder).label(label))
}

fn set_text(
    field: &Entity<InputField>,
    value: &str,
    window: &mut Window,
    cx: &mut Context<CopyTableForm>,
) {
    field.update(cx, |input, cx| input.set_text(value, window, cx));
}
