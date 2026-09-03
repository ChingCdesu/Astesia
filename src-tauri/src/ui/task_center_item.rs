use std::sync::Arc;

use gpui::{ClickEvent, Entity, FocusHandle, Subscription, Task};
use zed_ui::prelude::*;

use crate::{
    application::Application,
    platform::UiEvent,
    tasks::{BackgroundTask, TaskStatus},
};

use super::{localization::text, shell::ShellSettings};

pub(super) struct TaskCenterItem {
    application: Arc<Application>,
    tasks: Vec<BackgroundTask>,
    error: Option<String>,
    loading: bool,
    load_generation: u64,
    focus_handle: FocusHandle,
    settings: Entity<ShellSettings>,
    _settings_observation: Subscription,
    _application_events: Task<()>,
}

impl TaskCenterItem {
    pub(super) fn new(
        application: Arc<Application>,
        settings: Entity<ShellSettings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        let mut events = application.subscribe_events();
        let application_events = cx.spawn(async move |item, cx| loop {
            match events.recv().await {
                Ok(UiEvent::TaskProgress { .. } | UiEvent::TaskCompleted { .. }) => {
                    item.update(cx, |item, cx| item.refresh(cx)).ok();
                }
                Ok(UiEvent::McpConnectionsChanged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    item.update(cx, |item, cx| item.refresh(cx)).ok();
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        });
        let mut item = Self {
            application,
            tasks: Vec::new(),
            error: None,
            loading: false,
            load_generation: 0,
            focus_handle: cx.focus_handle(),
            settings,
            _settings_observation: settings_observation,
            _application_events: application_events,
        };
        item.refresh(cx);
        item
    }

    pub(super) fn label(&self) -> String {
        let active = self
            .tasks
            .iter()
            .filter(|task| !task.status.is_terminal())
            .count();
        if active == 0 {
            "Tasks".to_string()
        } else {
            format!("Tasks ({active})")
        }
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
    }

    fn refresh_click(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load_generation = self.load_generation.saturating_add(1);
        let generation = self.load_generation;
        self.loading = true;
        cx.notify();
        let tasks = self.application.tasks().clone();
        let load = gpui_tokio::Tokio::spawn(cx, async move { tasks.list_tasks().await });
        cx.spawn(async move |item, cx| {
            let result = load.await.map_err(|error| error.to_string());
            item.update(cx, |item, cx| {
                if item.load_generation != generation {
                    return;
                }
                item.loading = false;
                match result {
                    Ok(tasks) => {
                        item.tasks = tasks;
                        item.error = None;
                    }
                    Err(error) => item.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn cancel(&mut self, id: String, cx: &mut Context<Self>) {
        let manager = self.application.tasks().clone();
        let cancel = gpui_tokio::Tokio::spawn(cx, async move { manager.cancel_task(&id).await });
        cx.spawn(async move |item, cx| {
            let result = match cancel.await {
                Ok(result) => result,
                Err(error) => Err(error.to_string()),
            };
            item.update(cx, |item, cx| {
                if let Err(error) = result {
                    item.error = Some(error);
                }
                item.refresh(cx);
            })
            .ok();
        })
        .detach();
    }
}

impl Render for TaskCenterItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let language = self.settings.read(cx).language();
        let content = if self.tasks.is_empty() && !self.loading {
            v_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .gap_1()
                .child(Label::new(text(
                    language,
                    "尚无后台任务",
                    "No background tasks yet",
                )))
                .child(
                    Label::new(text(
                        language,
                        "备份、恢复、复制和导出任务会显示在这里。",
                        "Backup, restore, copy, and export tasks will appear here.",
                    ))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
                )
                .into_any_element()
        } else {
            v_flex()
                .id("background-task-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .children(self.tasks.iter().enumerate().map(|(index, task)| {
                    let id = task.id.clone();
                    let terminal = task.status.is_terminal();
                    let progress = (task.progress.clamp(0.0, 1.0) * 100.0).round() as u32;
                    v_flex()
                        .id(("background-task-row", index))
                        .flex_none()
                        .gap_1()
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(colors.border)
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Label::new(task.name.clone())
                                        .size(LabelSize::Small)
                                        .weight(gpui::FontWeight::MEDIUM)
                                        .truncate()
                                        .flex_1(),
                                )
                                .child(
                                    Label::new(status_label(&task.status, language))
                                        .size(LabelSize::XSmall)
                                        .color(status_color(&task.status)),
                                )
                                .when(!terminal, |element| {
                                    element.child(
                                        Button::new(
                                            format!("cancel-background-task-{index}"),
                                            text(language, "取消", "Cancel"),
                                        )
                                        .size(ButtonSize::Compact)
                                        .disabled(task.status == TaskStatus::Cancelling)
                                        .on_click(
                                            cx.listener(move |item, _, _, cx| {
                                                item.cancel(id.clone(), cx);
                                            }),
                                        ),
                                    )
                                }),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Label::new(format!("{progress}%"))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                                .child(
                                    Label::new(task.message.clone())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate()
                                        .flex_1(),
                                ),
                        )
                }))
                .into_any_element()
        };

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("TaskCenterItem")
            .size_full()
            .overflow_hidden()
            .child(
                h_flex()
                    .h(px(44.0))
                    .flex_none()
                    .px_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(colors.border)
                    .bg(colors.panel_background)
                    .child(
                        Label::new(text(language, "后台任务", "Background Tasks"))
                            .size(LabelSize::Small)
                            .weight(gpui::FontWeight::MEDIUM)
                            .flex_1(),
                    )
                    .child(
                        Button::new(
                            "refresh-background-tasks",
                            text(language, "刷新", "Refresh"),
                        )
                        .size(ButtonSize::Compact)
                        .loading(self.loading)
                        .on_click(cx.listener(Self::refresh_click)),
                    ),
            )
            .when_some(self.error.clone(), |element, error| {
                element.child(
                    h_flex()
                        .px_3()
                        .py_1()
                        .border_b_1()
                        .border_color(colors.border)
                        .child(
                            Label::new(error)
                                .size(LabelSize::XSmall)
                                .color(Color::Error),
                        ),
                )
            })
            .child(content)
    }
}

fn status_label(status: &TaskStatus, language: crate::platform::UiLanguage) -> &'static str {
    match status {
        TaskStatus::Pending => text(language, "等待中", "Pending"),
        TaskStatus::Running => text(language, "运行中", "Running"),
        TaskStatus::Cancelling => text(language, "取消中", "Cancelling"),
        TaskStatus::Completed => text(language, "已完成", "Completed"),
        TaskStatus::Partial => text(language, "部分完成", "Partial"),
        TaskStatus::Failed => text(language, "失败", "Failed"),
        TaskStatus::Cancelled => text(language, "已取消", "Cancelled"),
    }
}

fn status_color(status: &TaskStatus) -> Color {
    match status {
        TaskStatus::Completed => Color::Success,
        TaskStatus::Partial | TaskStatus::Cancelling => Color::Warning,
        TaskStatus::Failed => Color::Error,
        TaskStatus::Pending | TaskStatus::Running | TaskStatus::Cancelled => Color::Muted,
    }
}
