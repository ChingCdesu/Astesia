use std::sync::Arc;

use editor::Editor;
use gpui::{rgb, ClickEvent, Entity, Focusable as _, Subscription};
use workspace::ModalLayer;
use zed_ui::prelude::*;

use crate::application::connection_workspace::ConnectionWorkspaceError;
use crate::application::Application;

use super::{
    connection_profile_form::{
        ConnectionProfileForm, ConnectionProfileFormMode, ConnectionProfileSaved,
    },
    connections::{
        ConnectionActivityStatus, ConnectionProfilesEvent, ConnectionProfilesPanel,
        ConnectionSessionStatus,
    },
};

pub(super) struct AstesiaRoot {
    phase: AppPhase,
    editor: Entity<Editor>,
}

enum AppPhase {
    Loading,
    Ready(Entity<AstesiaWorkspace>),
    Failed(ConnectionWorkspaceError),
}

impl AstesiaRoot {
    pub(super) fn new(editor: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut root = Self {
            phase: AppPhase::Loading,
            editor,
        };
        root.load_application(window, cx);
        root
    }

    fn retry(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.load_application(window, cx);
    }

    fn load_application(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.phase = AppPhase::Loading;
        cx.notify();

        let load = gpui_tokio::Tokio::spawn(cx, async move {
            Application::probe_native_state().await?;
            Application::new().map(Arc::new)
        });
        cx.spawn_in(window, async move |root, cx| {
            let result = match load.await {
                Ok(application) => application.map_err(ConnectionWorkspaceError::from),
                Err(error) => Err(ConnectionWorkspaceError::startup(error)),
            };
            root.update_in(cx, |root, window, cx| {
                root.phase = match result {
                    Ok(application) => {
                        let connection_profiles =
                            cx.new(|cx| ConnectionProfilesPanel::new(application.clone(), cx));
                        let workspace = cx.new(|cx| {
                            AstesiaWorkspace::new(
                                application,
                                connection_profiles,
                                root.editor.clone(),
                                window,
                                cx,
                            )
                        });
                        window.focus(&root.editor.read(cx).focus_handle(cx), cx);
                        AppPhase::Ready(workspace)
                    }
                    Err(error) => AppPhase::Failed(error),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for AstesiaRoot {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        match &self.phase {
            AppPhase::Loading => v_flex()
                .size_full()
                .justify_center()
                .items_center()
                .bg(colors.background)
                .text_color(colors.text)
                .child(Label::new("正在加载 Astesia…").size(LabelSize::Small))
                .into_any_element(),
            AppPhase::Ready(workspace) => workspace.clone().into_any_element(),
            AppPhase::Failed(error) => v_flex()
                .size_full()
                .justify_center()
                .items_center()
                .gap_3()
                .p_6()
                .text_center()
                .bg(colors.background)
                .text_color(colors.text)
                .child(Label::new(error.message.clone()).size(LabelSize::Small))
                .child(
                    Label::new(error.remediation.clone())
                        .size(LabelSize::XSmall)
                        .line_clamp(3),
                )
                .child(Label::new(format!("错误码：{}", error.code)).size(LabelSize::XSmall))
                .child(
                    Button::new("retry-application-startup", "重试")
                        .size(ButtonSize::Compact)
                        .on_click(cx.listener(Self::retry)),
                )
                .into_any_element(),
        }
    }
}

pub(super) struct AstesiaWorkspace {
    application: Arc<Application>,
    connection_profiles: Entity<ConnectionProfilesPanel>,
    editor: Entity<Editor>,
    modal_layer: Entity<ModalLayer>,
    _profiles_subscription: Subscription,
    _profiles_observation: Subscription,
    profile_form_subscription: Option<Subscription>,
}

impl AstesiaWorkspace {
    pub(super) fn new(
        application: Arc<Application>,
        connection_profiles: Entity<ConnectionProfilesPanel>,
        editor: Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let modal_layer = cx.new(|_| ModalLayer::new());
        let profiles_subscription = cx.subscribe_in(
            &connection_profiles,
            window,
            |workspace, _, event, window, cx| {
                workspace.handle_profiles_event(event, window, cx);
            },
        );
        let profiles_observation = cx.observe(&connection_profiles, |_, _, cx| cx.notify());
        Self {
            application,
            connection_profiles,
            editor,
            modal_layer,
            _profiles_subscription: profiles_subscription,
            _profiles_observation: profiles_observation,
            profile_form_subscription: None,
        }
    }

    fn handle_profiles_event(
        &mut self,
        event: &ConnectionProfilesEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = match event {
            ConnectionProfilesEvent::CreateRequested => ConnectionProfileFormMode::Create,
            ConnectionProfilesEvent::EditRequested(profile) => {
                ConnectionProfileFormMode::Edit(profile.clone())
            }
        };
        self.open_profile_form(mode, window, cx);
    }

    fn open_profile_form(
        &mut self,
        mode: ConnectionProfileFormMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let application = self.application.clone();
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle_modal(window, cx, move |window, cx| {
                ConnectionProfileForm::new(application, mode, window, cx)
            });
        });

        let Some(form) = self
            .modal_layer
            .read(cx)
            .active_modal::<ConnectionProfileForm>()
        else {
            return;
        };
        self.profile_form_subscription = Some(cx.subscribe_in(
            &form,
            window,
            |workspace, _, event: &ConnectionProfileSaved, _, cx| {
                workspace.connection_profiles.update(cx, |panel, cx| {
                    panel.profile_saved(event.profile.id.clone(), cx);
                });
            },
        ));
    }
}

impl Render for AstesiaWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let status = self.connection_profiles.read(cx).status();
        let status_indicator = if status.activity == ConnectionActivityStatus::NeedsRefresh {
            rgb(0xef4444)
        } else {
            match status.session {
                ConnectionSessionStatus::Connected => rgb(0x22c55e),
                ConnectionSessionStatus::Connecting
                | ConnectionSessionStatus::Disconnecting
                | ConnectionSessionStatus::Deleting => rgb(0xeab308),
                ConnectionSessionStatus::Loading
                | ConnectionSessionStatus::NoSelection
                | ConnectionSessionStatus::Disconnected => rgb(0xa1a1aa),
            }
        };

        v_flex()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .overflow_hidden()
                    .child(self.connection_profiles.clone())
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .min_h_0()
                            .child(
                                h_flex()
                                    .h(px(40.0))
                                    .flex_none()
                                    .items_end()
                                    .px_1()
                                    .border_b_1()
                                    .border_color(colors.border)
                                    .bg(colors.tab_bar_background)
                                    .child(
                                        h_flex()
                                            .h(px(36.0))
                                            .px_3()
                                            .items_center()
                                            .rounded_t_md()
                                            .border_1()
                                            .border_b_0()
                                            .border_color(colors.border)
                                            .bg(colors.tab_active_background)
                                            .child(
                                                Label::new("查询")
                                                    .size(LabelSize::Small)
                                                    .weight(gpui::FontWeight::MEDIUM),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .w_full()
                                    .min_h_0()
                                    .min_w_0()
                                    .bg(colors.background)
                                    .child(self.editor.clone()),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .h(px(32.0))
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .border_t_1()
                    .border_color(colors.border)
                    .bg(colors.status_bar_background)
                    .child(div().size(px(8.0)).rounded_full().bg(status_indicator))
                    .child(Label::new(status.summary).size(LabelSize::XSmall))
                    .child(div().flex_1())
                    .child(Label::new(status.session.label()).size(LabelSize::XSmall))
                    .child(Label::new(status.activity.label()).size(LabelSize::XSmall)),
            )
            .child(self.modal_layer.clone())
    }
}
