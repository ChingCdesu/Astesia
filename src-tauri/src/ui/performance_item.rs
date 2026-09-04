use std::{sync::Arc, time::Duration};

use gpui::{ClickEvent, Entity, FocusHandle, Subscription, Task};
use zed_ui::prelude::*;

mod presentation;

use presentation::{render_dashboard_content, render_refresh_error};

use super::{engine_presentation::engine_color, localization::text, shell::ShellSettings};
use crate::application::{
    Application, PerformanceDashboardState, PerformanceLoadApply, PerformanceRefreshInterval,
    QueryTarget,
};
use crate::platform::UiLanguage;

pub(super) struct PerformanceItem {
    application: Arc<Application>,
    state: PerformanceDashboardState,
    auto_refresh: bool,
    refresh_interval: PerformanceRefreshInterval,
    auto_refresh_generation: u64,
    auto_refresh_task: Option<Task<()>>,
    focus_handle: FocusHandle,
    settings: Entity<ShellSettings>,
    _settings_observation: Subscription,
}

impl PerformanceItem {
    pub(super) fn new(
        application: Arc<Application>,
        target: QueryTarget,
        settings: Entity<ShellSettings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        let mut item = Self {
            application,
            state: PerformanceDashboardState::new(target),
            auto_refresh: false,
            refresh_interval: PerformanceRefreshInterval::Ten,
            auto_refresh_generation: 0,
            auto_refresh_task: None,
            focus_handle: cx.focus_handle(),
            settings,
            _settings_observation: settings_observation,
        };
        item.refresh(cx);
        item
    }

    pub(super) fn label(&self, cx: &App) -> String {
        let language = self.settings.read(cx).language();
        format!(
            "{} [{}]",
            text(language, "性能", "Performance"),
            self.state.target().database
        )
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
    }

    pub(super) fn invalidate_session(
        &mut self,
        connection_id: &str,
        session_generation: u64,
        cx: &mut Context<Self>,
    ) {
        if self
            .state
            .invalidate_session(connection_id, session_generation)
        {
            self.auto_refresh = false;
            self.stop_auto_refresh();
            cx.notify();
        }
    }

    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self.state.begin_load() else {
            return;
        };
        cx.notify();
        let service = self.application.performance().clone();
        let target = request.target().clone();
        let load = gpui_tokio::Tokio::spawn(cx, async move { service.metrics(&target).await });
        cx.spawn(async move |item, cx| {
            let result = match load.await {
                Ok(result) => result,
                Err(error) => Err(format!("Performance refresh ended unexpectedly: {error}")),
            };
            item.update(cx, |item, cx| {
                if item.state.finish_load(request, result) == PerformanceLoadApply::Applied {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn refresh_click(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
    }

    fn toggle_auto_refresh(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.state.is_available() {
            return;
        }
        self.auto_refresh = !self.auto_refresh;
        self.restart_auto_refresh(cx);
        cx.notify();
    }

    fn select_refresh_interval(
        &mut self,
        interval: PerformanceRefreshInterval,
        cx: &mut Context<Self>,
    ) {
        if self.refresh_interval == interval {
            return;
        }
        self.refresh_interval = interval;
        if self.auto_refresh {
            self.restart_auto_refresh(cx);
        }
        cx.notify();
    }

    fn restart_auto_refresh(&mut self, cx: &mut Context<Self>) {
        self.stop_auto_refresh();
        if !self.auto_refresh || !self.state.is_available() {
            return;
        }
        let generation = self.auto_refresh_generation;
        let interval = Duration::from_secs(self.refresh_interval.seconds());
        self.auto_refresh_task = Some(cx.spawn(async move |item, cx| loop {
            cx.background_executor().timer(interval).await;
            let keep_running = item
                .update(cx, |item, cx| {
                    if !item.auto_refresh || item.auto_refresh_generation != generation {
                        return false;
                    }
                    item.refresh(cx);
                    true
                })
                .unwrap_or(false);
            if !keep_running {
                break;
            }
        }));
    }

    fn stop_auto_refresh(&mut self) {
        self.auto_refresh_generation = self.auto_refresh_generation.saturating_add(1);
        self.auto_refresh_task.take();
    }

    fn render_toolbar(
        &self,
        target: &QueryTarget,
        language: UiLanguage,
        loading: bool,
        available: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        h_flex()
            .h(px(40.0))
            .flex_none()
            .gap_2()
            .px_3()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.panel_background)
            .child(
                div()
                    .size(px(8.0))
                    .rounded_full()
                    .bg(engine_color(target.db_type)),
            )
            .child(
                Label::new(text(language, "性能监控", "Performance Monitor"))
                    .size(LabelSize::Small)
                    .weight(gpui::FontWeight::SEMIBOLD),
            )
            .child(
                Label::new(format!("{} · {}", target.connection_name, target.database))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted)
                    .truncate(),
            )
            .child(div().flex_1())
            .when(self.auto_refresh, |element| {
                element.children(PerformanceRefreshInterval::ALL.map(|interval| {
                    Button::new(
                        format!("performance-refresh-{}", interval.seconds()),
                        format!("{}s", interval.seconds()),
                    )
                    .size(ButtonSize::Compact)
                    .toggle_state(self.refresh_interval == interval)
                    .on_click(cx.listener(move |item, _, _, cx| {
                        item.select_refresh_interval(interval, cx);
                    }))
                }))
            })
            .child(
                Button::new(
                    "toggle-performance-auto-refresh",
                    text(language, "自动刷新", "Auto Refresh"),
                )
                .size(ButtonSize::Compact)
                .toggle_state(self.auto_refresh)
                .disabled(!available)
                .on_click(cx.listener(Self::toggle_auto_refresh)),
            )
            .child(
                Button::new("refresh-performance", text(language, "刷新", "Refresh"))
                    .size(ButtonSize::Compact)
                    .start_icon(Icon::new(IconName::RotateCw).size(IconSize::XSmall))
                    .loading(loading)
                    .disabled(loading || !available)
                    .on_click(cx.listener(Self::refresh_click)),
            )
            .into_any_element()
    }
}

impl Render for PerformanceItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let language = self.settings.read(cx).language();
        let target = self.state.target().clone();
        let snapshot = self.state.snapshot().cloned();
        let error = self.state.error().map(str::to_string);
        let loading = self.state.is_loading();
        let available = self.state.is_available();
        let has_snapshot = snapshot.is_some();
        let content = render_dashboard_content(
            snapshot.as_ref(),
            error.as_deref(),
            loading,
            target.db_type,
            language,
            cx,
        );

        v_flex()
            .key_context("PerformanceItem")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_h_0()
            .bg(colors.background)
            .child(self.render_toolbar(&target, language, loading, available, cx))
            .when_some(error.filter(|_| has_snapshot), |element, error| {
                element.child(render_refresh_error(error, cx))
            })
            .child(content)
    }
}
