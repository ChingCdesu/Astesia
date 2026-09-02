use std::sync::Arc;

use editor::Editor;
use gpui::{
    actions, rgb, App, ClickEvent, Entity, Focusable as _, PromptButton, PromptLevel, Subscription,
};
use workspace::ModalLayer;
use zed_ui::{prelude::*, Tooltip};

use crate::application::connection_workspace::ConnectionWorkspaceError;
use crate::application::Application;
use crate::platform::{DesktopPreferences, NativePreferencesStore, ThemePreference, UiLanguage};

use super::{
    command_palette::{CommandPalette, CommandSelected, WorkspaceCommand},
    connection_profile_form::{
        ConnectionProfileForm, ConnectionProfileFormMode, ConnectionProfileSaved,
    },
    connections::{
        ConnectionActivityStatus, ConnectionProfilesEvent, ConnectionProfilesPanel,
        ConnectionSessionStatus,
    },
    localization::{text, theme_label},
    query_item::{QueryDocumentStateChanged, QueryItem},
    shell::{
        notify_preference_error, refresh_active_theme, NotificationCenter, NotificationTone,
        ShellSettings,
    },
    sql_language,
    tabs::{QueryTabId, QueryTabsModel},
};

actions!(
    astesia_workspace,
    [
        ToggleCommandPalette,
        NewQueryTab,
        CloseActiveQueryTab,
        NextQueryTab,
        PreviousQueryTab,
        ToggleConnectionsSidebar,
        RefreshConnectionProfiles
    ]
);

pub(super) fn bind_workspace_keys(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new(
            "cmd-shift-p",
            ToggleCommandPalette,
            Some("AstesiaWorkspace"),
        ),
        gpui::KeyBinding::new(
            "ctrl-shift-p",
            ToggleCommandPalette,
            Some("AstesiaWorkspace"),
        ),
        gpui::KeyBinding::new("cmd-n", NewQueryTab, Some("AstesiaWorkspace")),
        gpui::KeyBinding::new("ctrl-n", NewQueryTab, Some("AstesiaWorkspace")),
        gpui::KeyBinding::new("cmd-w", CloseActiveQueryTab, Some("AstesiaWorkspace")),
        gpui::KeyBinding::new("ctrl-w", CloseActiveQueryTab, Some("AstesiaWorkspace")),
        gpui::KeyBinding::new("ctrl-tab", NextQueryTab, Some("AstesiaWorkspace")),
        gpui::KeyBinding::new("ctrl-shift-tab", PreviousQueryTab, Some("AstesiaWorkspace")),
        gpui::KeyBinding::new("cmd-b", ToggleConnectionsSidebar, Some("AstesiaWorkspace")),
        gpui::KeyBinding::new("ctrl-b", ToggleConnectionsSidebar, Some("AstesiaWorkspace")),
        gpui::KeyBinding::new("cmd-r", RefreshConnectionProfiles, Some("AstesiaWorkspace")),
        gpui::KeyBinding::new(
            "ctrl-r",
            RefreshConnectionProfiles,
            Some("AstesiaWorkspace"),
        ),
        gpui::KeyBinding::new("up", menu::SelectPrevious, Some("CommandPalette > Editor")),
        gpui::KeyBinding::new("down", menu::SelectNext, Some("CommandPalette > Editor")),
        gpui::KeyBinding::new("enter", menu::Confirm, Some("CommandPalette > Editor")),
        gpui::KeyBinding::new("escape", menu::Cancel, Some("CommandPalette")),
        gpui::KeyBinding::new("enter", menu::Confirm, Some("CommandPaletteEntry")),
        gpui::KeyBinding::new("space", menu::Confirm, Some("CommandPaletteEntry")),
        gpui::KeyBinding::new("enter", menu::Confirm, Some("QueryTabRow")),
        gpui::KeyBinding::new("space", menu::Confirm, Some("QueryTabRow")),
    ]);
}

pub(super) struct AstesiaRoot {
    phase: AppPhase,
    editor: Entity<Editor>,
    preferences: DesktopPreferences,
    preferences_store: Option<NativePreferencesStore>,
    preferences_warning: Option<String>,
    _appearance_subscription: Subscription,
}

enum AppPhase {
    Loading,
    Ready(Entity<AstesiaWorkspace>),
    Failed(ConnectionWorkspaceError),
}

impl AstesiaRoot {
    pub(super) fn new(
        editor: Entity<Editor>,
        preferences: DesktopPreferences,
        preferences_store: Option<NativePreferencesStore>,
        preferences_warning: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let appearance_subscription = cx.observe_window_appearance(window, |root, window, cx| {
            let theme_preference = match &root.phase {
                AppPhase::Ready(workspace) => workspace.read(cx).settings.read(cx).theme(),
                AppPhase::Loading | AppPhase::Failed(_) => root.preferences.theme,
            };
            *theme::SystemAppearance::global_mut(cx) =
                theme::SystemAppearance(window.appearance().into());
            if theme_preference == ThemePreference::System {
                refresh_active_theme(theme_preference, cx);
            }
        });
        let mut root = Self {
            phase: AppPhase::Loading,
            editor,
            preferences,
            preferences_store,
            preferences_warning,
            _appearance_subscription: appearance_subscription,
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
                        let settings = cx.new(|_| {
                            ShellSettings::new(
                                root.preferences.clone(),
                                root.preferences_store.clone(),
                            )
                        });
                        let notifications = cx.new(|_| NotificationCenter::new());
                        if let Some(warning) = root.preferences_warning.take() {
                            notifications.update(cx, |center, cx| {
                                center.push(NotificationTone::Warning, warning, cx);
                            });
                        }
                        let connection_profiles = cx.new(|cx| {
                            ConnectionProfilesPanel::new(application.clone(), settings.clone(), cx)
                        });
                        let workspace = cx.new(|cx| {
                            AstesiaWorkspace::new(
                                application,
                                connection_profiles,
                                root.editor.clone(),
                                settings,
                                notifications,
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
        let language = self.preferences.language;
        match &self.phase {
            AppPhase::Loading => v_flex()
                .size_full()
                .justify_center()
                .items_center()
                .bg(colors.background)
                .text_color(colors.text)
                .child(
                    Label::new(text(language, "正在加载 Astesia…", "Loading Astesia…"))
                        .size(LabelSize::Small),
                )
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
                .child(
                    Label::new(format!(
                        "{}{}",
                        text(language, "错误码：", "Error code: "),
                        error.code
                    ))
                    .size(LabelSize::XSmall),
                )
                .child(
                    Button::new("retry-application-startup", text(language, "重试", "Retry"))
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
    tabs: QueryTabsModel,
    query_tabs: Vec<QueryTab>,
    settings: Entity<ShellSettings>,
    notifications: Entity<NotificationCenter>,
    modal_layer: Entity<ModalLayer>,
    _profiles_subscription: Subscription,
    _profiles_observation: Subscription,
    _settings_observation: Subscription,
    profile_form_subscription: Option<Subscription>,
    command_palette_subscription: Option<Subscription>,
}

struct QueryTab {
    id: QueryTabId,
    item: Entity<QueryItem>,
    _document_subscription: Subscription,
}

impl AstesiaWorkspace {
    pub(super) fn new(
        application: Arc<Application>,
        connection_profiles: Entity<ConnectionProfilesPanel>,
        editor: Entity<Editor>,
        settings: Entity<ShellSettings>,
        notifications: Entity<NotificationCenter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let modal_layer = cx.new(|_| ModalLayer::new());
        let tabs = QueryTabsModel::new();
        let query_item =
            cx.new(|cx| QueryItem::new(application.clone(), editor, settings.clone(), cx));
        let query_item_subscription = cx
            .subscribe(&query_item, |_, _, _: &QueryDocumentStateChanged, cx| {
                cx.notify()
            });
        let query_tabs = vec![QueryTab {
            id: tabs.active(),
            item: query_item,
            _document_subscription: query_item_subscription,
        }];
        let profiles_subscription = cx.subscribe_in(
            &connection_profiles,
            window,
            |workspace, _, event, window, cx| {
                workspace.handle_profiles_event(event, window, cx);
            },
        );
        let profiles_observation = cx.observe(&connection_profiles, |_, _, cx| cx.notify());
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        Self {
            application,
            connection_profiles,
            tabs,
            query_tabs,
            settings,
            notifications,
            modal_layer,
            _profiles_subscription: profiles_subscription,
            _profiles_observation: profiles_observation,
            _settings_observation: settings_observation,
            profile_form_subscription: None,
            command_palette_subscription: None,
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
            ConnectionProfilesEvent::QueryTargetSelected(target) => {
                self.active_query_item().update(cx, |item, cx| {
                    item.set_target(Some(target.clone()), window, cx);
                });
                return;
            }
            ConnectionProfilesEvent::QueryTargetInvalidated(target) => {
                for tab in &self.query_tabs {
                    tab.item
                        .update(cx, |item, cx| item.invalidate_target(target, cx));
                }
                return;
            }
            ConnectionProfilesEvent::QuerySessionInvalidated {
                connection_id,
                session_generation,
            } => {
                for tab in &self.query_tabs {
                    tab.item.update(cx, |item, cx| {
                        item.invalidate_session(connection_id, *session_generation, cx)
                    });
                }
                return;
            }
            ConnectionProfilesEvent::QuerySessionsChanged(snapshot) => {
                for tab in &self.query_tabs {
                    tab.item.update(cx, |item, cx| {
                        item.reconcile_sessions(snapshot, cx);
                    });
                }
                return;
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
        let language = self.settings.read(cx).language();
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle_modal(window, cx, move |window, cx| {
                ConnectionProfileForm::new(application, mode, language, window, cx)
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
                let language = workspace.settings.read(cx).language();
                workspace.notifications.update(cx, |center, cx| {
                    center.push(
                        NotificationTone::Info,
                        format!(
                            "{}: {}",
                            text(language, "连接配置已保存", "Connection profile saved"),
                            event.profile.name
                        ),
                        cx,
                    );
                });
            },
        ));
    }

    fn active_query_item(&self) -> &Entity<QueryItem> {
        let active = self.tabs.active();
        &self
            .query_tabs
            .iter()
            .find(|tab| tab.id == active)
            .expect("active query tab must have a view")
            .item
    }

    fn focus_active_query(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_query_item()
            .update(cx, |item, cx| item.focus(window, cx));
        cx.notify();
    }

    fn has_active_modal(&self, cx: &App) -> bool {
        self.modal_layer.read(cx).has_active_modal()
    }

    fn new_query_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = cx.new(|cx| sql_language::editor(super::INITIAL_QUERY, window, cx));
        let target = self.connection_profiles.read(cx).query_target().cloned();
        let item = cx
            .new(|cx| QueryItem::new(self.application.clone(), editor, self.settings.clone(), cx));
        let document_subscription =
            cx.subscribe(&item, |_, _, _: &QueryDocumentStateChanged, cx| cx.notify());
        if let Some(target) = target {
            item.update(cx, |item, cx| item.set_target(Some(target), window, cx));
        } else {
            item.update(cx, |item, cx| item.focus(window, cx));
        }
        let id = self.tabs.add();
        self.query_tabs.push(QueryTab {
            id,
            item,
            _document_subscription: document_subscription,
        });
        cx.notify();
    }

    fn activate_tab(&mut self, id: QueryTabId, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.activate(id) {
            self.focus_active_query(window, cx);
        }
    }

    fn close_tab(&mut self, id: QueryTabId, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.tabs().len() == 1 {
            return;
        }
        let Some(tab) = self.query_tabs.iter().find(|tab| tab.id == id) else {
            return;
        };
        if tab.item.read(cx).has_unsaved_changes() {
            self.confirm_discard_and_close(id, window, cx);
            return;
        }
        self.close_tab_now(id, window, cx);
    }

    fn confirm_discard_and_close(
        &mut self,
        id: QueryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_prompt() {
            return;
        }
        let Some(tab) = self.query_tabs.iter().find(|tab| tab.id == id) else {
            return;
        };
        let language = self.settings.read(cx).language();
        let name = tab
            .item
            .read(cx)
            .file_display_name()
            .unwrap_or_else(|| text(language, "未命名查询", "Untitled Query").to_string());
        let message = format!(
            "{} “{name}”?",
            text(language, "放弃更改并关闭", "Discard changes and close")
        );
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            Some(text(
                language,
                "未保存的更改将会丢失。",
                "Unsaved changes will be lost.",
            )),
            &[
                PromptButton::ok(text(language, "放弃并关闭", "Discard and Close")),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |workspace, cx| {
            if answer.await.ok() != Some(0) {
                return;
            }
            workspace
                .update_in(cx, |workspace, window, cx| {
                    workspace.close_tab_now(id, window, cx);
                })
                .ok();
        })
        .detach();
    }

    fn close_tab_now(&mut self, id: QueryTabId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.close(id) {
            return;
        }
        self.query_tabs.retain(|tab| tab.id != id);
        self.focus_active_query(window, cx);
    }

    fn next_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.next();
        self.focus_active_query(window, cx);
    }

    fn previous_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.previous();
        self.focus_active_query(window, cx);
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        let result = self
            .settings
            .update(cx, |settings, cx| settings.toggle_sidebar(cx));
        if let Err(error) = result {
            notify_preference_error(&self.notifications, error, cx);
        }
    }

    fn set_theme(&mut self, theme: ThemePreference, cx: &mut Context<Self>) {
        let result = self
            .settings
            .update(cx, |settings, cx| settings.set_theme(theme, cx));
        if let Err(error) = result {
            notify_preference_error(&self.notifications, error, cx);
        }
    }

    fn set_language(&mut self, language: UiLanguage, cx: &mut Context<Self>) {
        let result = self
            .settings
            .update(cx, |settings, cx| settings.set_language(language, cx));
        if let Err(error) = result {
            notify_preference_error(&self.notifications, error, cx);
        }
    }

    fn execute_command(
        &mut self,
        command: WorkspaceCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            WorkspaceCommand::NewQuery => self.new_query_tab(window, cx),
            WorkspaceCommand::CloseActiveTab => self.close_tab(self.tabs.active(), window, cx),
            WorkspaceCommand::NextTab => self.next_tab(window, cx),
            WorkspaceCommand::PreviousTab => self.previous_tab(window, cx),
            WorkspaceCommand::ToggleSidebar => self.toggle_sidebar(cx),
            WorkspaceCommand::RefreshConnections => {
                self.connection_profiles
                    .update(cx, |panel, cx| panel.refresh_profiles(cx));
            }
            WorkspaceCommand::SetTheme(theme) => self.set_theme(theme, cx),
            WorkspaceCommand::SetLanguage(language) => self.set_language(language, cx),
        }
    }

    fn toggle_command_palette(
        &mut self,
        _: &ToggleCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_layer.read(cx).has_active_modal()
            && self
                .modal_layer
                .read(cx)
                .active_modal::<CommandPalette>()
                .is_none()
        {
            return;
        }
        let language = self.settings.read(cx).language();
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle_modal(window, cx, move |window, cx| {
                CommandPalette::new(language, window, cx)
            });
        });
        let Some(palette) = self.modal_layer.read(cx).active_modal::<CommandPalette>() else {
            self.command_palette_subscription = None;
            return;
        };
        self.command_palette_subscription = Some(cx.subscribe_in(
            &palette,
            window,
            |workspace, _, event: &CommandSelected, window, cx| {
                workspace
                    .modal_layer
                    .update(cx, |layer, cx| layer.hide_modal(window, cx));
                workspace.execute_command(event.0, window, cx);
            },
        ));
    }

    fn new_query_action(&mut self, _: &NewQueryTab, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_active_modal(cx) {
            self.new_query_tab(window, cx);
        }
    }

    fn close_active_tab_action(
        &mut self,
        _: &CloseActiveQueryTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.has_active_modal(cx) {
            self.close_tab(self.tabs.active(), window, cx);
        }
    }

    fn next_tab_action(&mut self, _: &NextQueryTab, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_active_modal(cx) {
            self.next_tab(window, cx);
        }
    }

    fn previous_tab_action(
        &mut self,
        _: &PreviousQueryTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.has_active_modal(cx) {
            self.previous_tab(window, cx);
        }
    }

    fn toggle_sidebar_action(
        &mut self,
        _: &ToggleConnectionsSidebar,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.has_active_modal(cx) {
            self.toggle_sidebar(cx);
        }
    }

    fn refresh_connections_action(
        &mut self,
        _: &RefreshConnectionProfiles,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.has_active_modal(cx) {
            self.connection_profiles
                .update(cx, |panel, cx| panel.refresh_profiles(cx));
        }
    }

    fn open_palette_click(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_command_palette(&ToggleCommandPalette, window, cx);
    }

    fn cycle_theme(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let theme = self.settings.read(cx).theme().next();
        self.set_theme(theme, cx);
    }

    fn cycle_language(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let language = self.settings.read(cx).language().next();
        self.set_language(language, cx);
    }
}

impl Render for AstesiaWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let status = self.connection_profiles.read(cx).status(cx);
        let language = self.settings.read(cx).language();
        let theme = self.settings.read(cx).theme();
        let sidebar_visible = self.settings.read(cx).sidebar_visible();
        let active_item = self.active_query_item().clone();
        let active_tab = self.tabs.active();
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
        let session_label = match status.session {
            ConnectionSessionStatus::Loading => {
                text(language, "连接状态加载中", "Loading connection state")
            }
            ConnectionSessionStatus::NoSelection => {
                text(language, "未选择连接", "No connection selected")
            }
            ConnectionSessionStatus::Disconnected => text(language, "未连接", "Disconnected"),
            ConnectionSessionStatus::Connecting => text(language, "连接中", "Connecting"),
            ConnectionSessionStatus::Connected => text(language, "已连接", "Connected"),
            ConnectionSessionStatus::Disconnecting => text(language, "断开中", "Disconnecting"),
            ConnectionSessionStatus::Deleting => text(language, "删除中", "Deleting"),
        };
        let activity_label = match status.activity {
            ConnectionActivityStatus::Loading => text(language, "正在加载", "Loading"),
            ConnectionActivityStatus::Refreshing => text(language, "正在刷新", "Refreshing"),
            ConnectionActivityStatus::LoadingDatabases => {
                text(language, "正在加载数据库", "Loading databases")
            }
            ConnectionActivityStatus::LoadingObjects => {
                text(language, "正在加载对象", "Loading objects")
            }
            ConnectionActivityStatus::Working => text(language, "正在处理", "Working"),
            ConnectionActivityStatus::NeedsRefresh => {
                text(language, "需要刷新", "Refresh required")
            }
            ConnectionActivityStatus::Ready => text(language, "就绪", "Ready"),
        };

        v_flex()
            .key_context("AstesiaWorkspace")
            .on_action(cx.listener(Self::toggle_command_palette))
            .on_action(cx.listener(Self::new_query_action))
            .on_action(cx.listener(Self::close_active_tab_action))
            .on_action(cx.listener(Self::next_tab_action))
            .on_action(cx.listener(Self::previous_tab_action))
            .on_action(cx.listener(Self::toggle_sidebar_action))
            .on_action(cx.listener(Self::refresh_connections_action))
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
                    .when(sidebar_visible, |element| {
                        element.child(self.connection_profiles.clone())
                    })
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
                                    .gap_1()
                                    .children(self.tabs.tabs().iter().enumerate().map(
                                        |(index, id)| {
                                            let id = *id;
                                            let is_active = id == active_tab;
                                            let tab = self
                                                .query_tabs
                                                .iter()
                                                .find(|tab| tab.id == id)
                                                .expect("query tab model and views must agree");
                                            let item = tab.item.read(cx);
                                            let fallback = format!(
                                                "{} {}",
                                                text(language, "查询", "Query"),
                                                index + 1
                                            );
                                            let label = item.document_label(&fallback);
                                            h_flex()
                                                .id(format!("query-tab-{index}"))
                                                .role(gpui::Role::Button)
                                                .tab_index(0)
                                                .key_context("QueryTabRow")
                                                .aria_label(label.clone())
                                                .aria_toggled(if is_active {
                                                    gpui::Toggled::True
                                                } else {
                                                    gpui::Toggled::False
                                                })
                                                .h(px(36.0))
                                                .min_w(px(112.0))
                                                .px_2()
                                                .gap_1()
                                                .items_center()
                                                .rounded_t_md()
                                                .border_1()
                                                .border_b_0()
                                                .border_color(colors.border)
                                                .bg(if is_active {
                                                    colors.tab_active_background
                                                } else {
                                                    colors.tab_inactive_background
                                                })
                                                .cursor_pointer()
                                                .on_action(cx.listener(
                                                    move |workspace, _: &menu::Confirm, window, cx| {
                                                        workspace.activate_tab(id, window, cx);
                                                    },
                                                ))
                                                .on_click(cx.listener(
                                                    move |workspace, _, window, cx| {
                                                        workspace.activate_tab(id, window, cx);
                                                    },
                                                ))
                                                .child(
                                                    Label::new(label)
                                                    .size(LabelSize::Small)
                                                    .weight(gpui::FontWeight::MEDIUM)
                                                    .flex_1(),
                                                )
                                                .when(self.tabs.tabs().len() > 1, |element| {
                                                    element.child(
                                                        IconButton::new(
                                                            format!("close-query-tab-{index}"),
                                                            IconName::Close,
                                                        )
                                                        .icon_size(IconSize::XSmall)
                                                        .on_click(cx.listener(
                                                            move |workspace, _, window, cx| {
                                                                workspace.close_tab(id, window, cx);
                                                            },
                                                        )),
                                                    )
                                                })
                                        },
                                    ))
                                    .child(
                                        IconButton::new("new-query-tab", IconName::Plus)
                                            .icon_size(IconSize::Small)
                                            .tooltip(move |_, cx| {
                                                Tooltip::with_meta(
                                                    text(language, "新建查询", "New Query"),
                                                    None,
                                                    "⌘N",
                                                    cx,
                                                )
                                            })
                                            .on_click(cx.listener(|workspace, _, window, cx| {
                                                workspace.new_query_tab(window, cx);
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .w_full()
                                    .min_h_0()
                                    .min_w_0()
                                    .bg(colors.background)
                                    .child(active_item),
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
                    .child(
                        Button::new("open-command-palette", text(language, "命令", "Commands"))
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Transparent)
                            .key_binding(zed_ui::KeyBinding::for_action(&ToggleCommandPalette, cx))
                            .on_click(cx.listener(Self::open_palette_click)),
                    )
                    .child(
                        Button::new("cycle-language", self.settings.read(cx).language().code())
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Transparent)
                            .on_click(cx.listener(Self::cycle_language)),
                    )
                    .child(
                        Button::new("cycle-theme", theme_label(language, theme))
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Transparent)
                            .on_click(cx.listener(Self::cycle_theme)),
                    )
                    .child(Label::new(session_label).size(LabelSize::XSmall))
                    .child(Label::new(activity_label).size(LabelSize::XSmall)),
            )
            .child(self.modal_layer.clone())
            .child(self.notifications.clone())
    }
}
