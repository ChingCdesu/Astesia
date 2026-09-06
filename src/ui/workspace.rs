use std::sync::Arc;

use crate::ui::components::{prelude::*, Indicator, Tooltip};
use crate::ui::modal::ModalLayer;
use gpui_kit::{
    actions, App, ClickEvent, Entity, FocusHandle, PromptButton, PromptLevel, Subscription,
};

use crate::application::connection_workspace::ConnectionWorkspaceError;
use crate::application::{Application, ConnectionWorkspaceSnapshot, QueryTarget};
use crate::db::TableRef;
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
    data_grid_item::DataGridItem,
    localization::{text, theme_label},
    object_definition_item::{ObjectDefinition, ObjectDefinitionItem},
    object_mutation_form::{ObjectMutationForm, ObjectMutationFormMode, ObjectMutationSaved},
    query_item::{QueryDocumentStateChanged, QueryItem},
    shell::{
        notify_preference_error, refresh_active_theme, NotificationCenter, NotificationTone,
        ShellSettings,
    },
    sql_language,
    table_structure_item::TableStructureItem,
    tabs::{WorkspaceTabId, WorkspaceTabsModel},
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
        gpui_kit::KeyBinding::new(
            "cmd-shift-p",
            ToggleCommandPalette,
            Some("AstesiaWorkspace"),
        ),
        gpui_kit::KeyBinding::new(
            "ctrl-shift-p",
            ToggleCommandPalette,
            Some("AstesiaWorkspace"),
        ),
        gpui_kit::KeyBinding::new("cmd-n", NewQueryTab, Some("AstesiaWorkspace")),
        gpui_kit::KeyBinding::new("ctrl-n", NewQueryTab, Some("AstesiaWorkspace")),
        gpui_kit::KeyBinding::new("cmd-w", CloseActiveQueryTab, Some("AstesiaWorkspace")),
        gpui_kit::KeyBinding::new("ctrl-w", CloseActiveQueryTab, Some("AstesiaWorkspace")),
        gpui_kit::KeyBinding::new("ctrl-tab", NextQueryTab, Some("AstesiaWorkspace")),
        gpui_kit::KeyBinding::new("ctrl-shift-tab", PreviousQueryTab, Some("AstesiaWorkspace")),
        gpui_kit::KeyBinding::new("cmd-b", ToggleConnectionsSidebar, Some("AstesiaWorkspace")),
        gpui_kit::KeyBinding::new("ctrl-b", ToggleConnectionsSidebar, Some("AstesiaWorkspace")),
        gpui_kit::KeyBinding::new("cmd-r", RefreshConnectionProfiles, Some("AstesiaWorkspace")),
        gpui_kit::KeyBinding::new(
            "ctrl-r",
            RefreshConnectionProfiles,
            Some("AstesiaWorkspace"),
        ),
        gpui_kit::KeyBinding::new("up", menu::SelectPrevious, Some("CommandPalette > Input")),
        gpui_kit::KeyBinding::new("down", menu::SelectNext, Some("CommandPalette > Input")),
        gpui_kit::KeyBinding::new("enter", menu::Confirm, Some("CommandPalette > Input")),
        gpui_kit::KeyBinding::new("escape", menu::Cancel, Some("CommandPalette")),
        gpui_kit::KeyBinding::new("enter", menu::Confirm, Some("CommandPaletteEntry")),
        gpui_kit::KeyBinding::new("space", menu::Confirm, Some("CommandPaletteEntry")),
        gpui_kit::KeyBinding::new("enter", menu::Confirm, Some("WorkspaceTabRow")),
        gpui_kit::KeyBinding::new("space", menu::Confirm, Some("WorkspaceTabRow")),
    ]);
}

mod item;
mod operations;
mod settings_menu;
mod tab_bar;
mod title_bar;
mod view;

use item::{WorkspaceItem, WorkspaceItemKey};

pub(super) struct AstesiaRoot {
    title_bar: Entity<title_bar::AstesiaTitleBar>,
    phase: AppPhase,
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
        preferences: DesktopPreferences,
        preferences_store: Option<NativePreferencesStore>,
        preferences_warning: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let appearance_subscription = cx.observe_window_appearance(window, |root, _, cx| {
            let theme_preference = match &root.phase {
                AppPhase::Ready(workspace) => workspace.read(cx).settings.read(cx).theme(),
                AppPhase::Loading | AppPhase::Failed(_) => root.preferences.theme,
            };
            if theme_preference == ThemePreference::System {
                refresh_active_theme(theme_preference, cx);
            }
        });
        let mut root = Self {
            title_bar: cx.new(|_| title_bar::AstesiaTitleBar),
            phase: AppPhase::Loading,
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

        let load = crate::ui::runtime::spawn(cx, async move {
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
                            ConnectionProfilesPanel::new(
                                application.clone(),
                                settings.clone(),
                                window,
                                cx,
                            )
                        });
                        let workspace = cx.new(|cx| {
                            AstesiaWorkspace::new(
                                application,
                                connection_profiles,
                                settings,
                                notifications,
                                window,
                                cx,
                            )
                        });
                        window.focus(&workspace.read(cx).focus_handle.clone(), cx);
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
        let content = match &self.phase {
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
        };

        v_flex()
            .size_full()
            .overflow_hidden()
            .child(self.title_bar.clone())
            .child(div().flex_1().min_h_0().child(content))
    }
}

pub(super) struct AstesiaWorkspace {
    application: Arc<Application>,
    connection_profiles: Entity<ConnectionProfilesPanel>,
    tabs: WorkspaceTabsModel,
    focus_handle: FocusHandle,
    tab_scroll_handle: gpui_kit::ScrollHandle,
    workspace_tabs: Vec<WorkspaceTab>,
    settings: Entity<ShellSettings>,
    notifications: Entity<NotificationCenter>,
    modal_layer: Entity<ModalLayer>,
    _profiles_subscription: Subscription,
    _profiles_observation: Subscription,
    _settings_observation: Subscription,
    _application_events: gpui_kit::Task<()>,
    profile_form_subscription: Option<Subscription>,
    object_mutation_form_subscription: Option<Subscription>,
    copy_table_form_subscription: Option<Subscription>,
    command_palette_subscription: Option<Subscription>,
}

struct WorkspaceTab {
    id: WorkspaceTabId,
    key: WorkspaceItemKey,
    item: WorkspaceItem,
    _subscriptions: Vec<Subscription>,
}

impl AstesiaWorkspace {
    pub(super) fn new(
        application: Arc<Application>,
        connection_profiles: Entity<ConnectionProfilesPanel>,
        settings: Entity<ShellSettings>,
        notifications: Entity<NotificationCenter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let modal_layer = cx.new(|_| ModalLayer::new());
        let tabs = WorkspaceTabsModel::new();
        let workspace_tabs = Vec::new();
        let profiles_subscription = cx.subscribe_in(
            &connection_profiles,
            window,
            |workspace, _, event, window, cx| {
                workspace.handle_profiles_event(event, window, cx);
            },
        );
        let profiles_observation = cx.observe(&connection_profiles, |_, _, cx| cx.notify());
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        let mut application_events = application.subscribe_events();
        let task_events = cx.spawn(async move |workspace, cx| loop {
            match application_events.recv().await {
                Ok(crate::platform::UiEvent::TaskCompleted { task }) => {
                    workspace
                        .update(cx, |workspace, cx| {
                            let tone = match &task.status {
                                crate::tasks::TaskStatus::Completed => NotificationTone::Info,
                                crate::tasks::TaskStatus::Partial
                                | crate::tasks::TaskStatus::Cancelled => NotificationTone::Warning,
                                crate::tasks::TaskStatus::Failed => NotificationTone::Error,
                                crate::tasks::TaskStatus::Pending
                                | crate::tasks::TaskStatus::Running
                                | crate::tasks::TaskStatus::Cancelling => return,
                            };
                            workspace.notifications.update(cx, |center, cx| {
                                center.push(tone, format!("{}: {}", task.name, task.message), cx);
                            });
                            cx.notify();
                        })
                        .ok();
                }
                Ok(
                    crate::platform::UiEvent::TaskProgress { .. }
                    | crate::platform::UiEvent::McpConnectionsChanged(_),
                ) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        });
        Self {
            application,
            connection_profiles,
            tabs,
            focus_handle: cx.focus_handle(),
            tab_scroll_handle: gpui_kit::ScrollHandle::new(),
            workspace_tabs,
            settings,
            notifications,
            modal_layer,
            _profiles_subscription: profiles_subscription,
            _profiles_observation: profiles_observation,
            _settings_observation: settings_observation,
            _application_events: task_events,
            profile_form_subscription: None,
            object_mutation_form_subscription: None,
            copy_table_form_subscription: None,
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
                if let Some(item) = self.active_query_item() {
                    item.update(cx, |item, cx| {
                        item.set_target(Some(target.clone()), window, cx);
                    });
                }
                return;
            }
            ConnectionProfilesEvent::TableStructureRequested { target, table } => {
                self.open_table_structure(target.clone(), table.clone(), window, cx);
                return;
            }
            ConnectionProfilesEvent::TableDataRequested { target, table } => {
                self.open_data_grid(target.clone(), table.clone(), window, cx);
                return;
            }
            ConnectionProfilesEvent::DocumentCollectionRequested { target, collection } => {
                self.open_document_collection(target.clone(), collection.clone(), window, cx);
                return;
            }
            ConnectionProfilesEvent::RedisKeyRequested { target, key } => {
                self.open_redis_key(target.clone(), key.clone(), window, cx);
                return;
            }
            ConnectionProfilesEvent::BackupRequested { target, tables } => {
                self.choose_backup_content(target.clone(), tables.clone(), window, cx);
                return;
            }
            ConnectionProfilesEvent::RestoreRequested { target } => {
                self.choose_restore_file(target.clone(), window, cx);
                return;
            }
            ConnectionProfilesEvent::PerformanceRequested { target } => {
                self.open_performance(target.clone(), window, cx);
                return;
            }
            ConnectionProfilesEvent::ErDiagramRequested { target } => {
                self.open_er_diagram(target.clone(), window, cx);
                return;
            }
            ConnectionProfilesEvent::CopyTableRequested {
                source,
                target,
                table,
            } => {
                self.open_copy_table_form(
                    source.clone(),
                    target.clone(),
                    table.clone(),
                    window,
                    cx,
                );
                return;
            }
            ConnectionProfilesEvent::ObjectDefinitionRequested(object) => {
                self.open_object_definition(object.clone(), window, cx);
                return;
            }
            ConnectionProfilesEvent::ObjectMutationRequested(mode) => {
                self.open_object_mutation_form(mode.clone(), window, cx);
                return;
            }
            ConnectionProfilesEvent::QueryTargetInvalidated(target) => {
                self.application
                    .query_completions()
                    .invalidate_session(&target.connection_id, target.session_generation);
                for tab in &self.workspace_tabs {
                    tab.item.invalidate_target(target, cx);
                }
                return;
            }
            ConnectionProfilesEvent::QuerySessionInvalidated {
                connection_id,
                session_generation,
            } => {
                self.application
                    .query_completions()
                    .invalidate_session(connection_id, *session_generation);
                for tab in &self.workspace_tabs {
                    tab.item
                        .invalidate_session(connection_id, *session_generation, cx);
                }
                return;
            }
            ConnectionProfilesEvent::QuerySessionsChanged(snapshot) => {
                self.application
                    .query_completions()
                    .retain_sessions(snapshot);
                for tab in &self.workspace_tabs {
                    tab.item.reconcile_sessions(snapshot, cx);
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

    fn open_object_mutation_form(
        &mut self,
        mode: ObjectMutationFormMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let application = self.application.clone();
        let language = self.settings.read(cx).language();
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle_modal(window, cx, move |window, cx| {
                ObjectMutationForm::new(application, mode, language, window, cx)
            });
        });
        let Some(form) = self
            .modal_layer
            .read(cx)
            .active_modal::<ObjectMutationForm>()
        else {
            return;
        };
        self.object_mutation_form_subscription = Some(cx.subscribe_in(
            &form,
            window,
            |workspace, _, event: &ObjectMutationSaved, _, cx| {
                workspace.connection_profiles.update(cx, |panel, cx| {
                    panel.object_mutated(
                        event.target.clone(),
                        event.kind == crate::application::DatabaseObjectKind::Database,
                        cx,
                    );
                });
                workspace.notifications.update(cx, |center, cx| {
                    center.push(
                        NotificationTone::Info,
                        format!(
                            "{}: {}",
                            text(
                                workspace.settings.read(cx).language(),
                                "数据库对象已更新",
                                "Database object updated"
                            ),
                            event.identity
                        ),
                        cx,
                    );
                });
            },
        ));
    }

    fn active_item(&self) -> Option<&WorkspaceItem> {
        let active = self.tabs.active()?;
        Some(
            &self
                .workspace_tabs
                .iter()
                .find(|tab| tab.id == active)
                .expect("active workspace tab must have a view")
                .item,
        )
    }

    fn active_query_item(&self) -> Option<&Entity<QueryItem>> {
        self.active_item()?.query()
    }

    fn open_or_activate(
        &mut self,
        key: WorkspaceItemKey,
        window: &mut Window,
        cx: &mut Context<Self>,
        build: impl FnOnce(
            &mut Self,
            &mut Window,
            &mut Context<Self>,
        ) -> (WorkspaceItem, Vec<Subscription>),
    ) {
        if let Some(id) = self
            .workspace_tabs
            .iter()
            .find(|tab| tab.key == key)
            .map(|tab| tab.id)
        {
            self.activate_tab(id, window, cx);
            return;
        }

        let (item, subscriptions) = build(self, window, cx);
        let id = self.tabs.add();
        self.workspace_tabs.push(WorkspaceTab {
            id,
            key,
            item,
            _subscriptions: subscriptions,
        });
        self.focus_active_item(window, cx);
    }

    fn focus_active_item(&self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .active_query_item()
            .and_then(|item| item.read(cx).query_target().cloned())
            .or_else(|| {
                self.workspace_tabs
                    .iter()
                    .find(|tab| Some(tab.id) == self.tabs.active())
                    .and_then(|tab| tab.key.target().cloned())
            });
        if let Some(target) = target {
            let table = self
                .workspace_tabs
                .iter()
                .find(|tab| Some(tab.id) == self.tabs.active())
                .and_then(|tab| match &tab.key {
                    WorkspaceItemKey::TableStructure(_, table)
                    | WorkspaceItemKey::DataGrid(_, table)
                    | WorkspaceItemKey::Document(_, table) => Some(table.clone()),
                    _ => None,
                });
            self.connection_profiles
                .update(cx, |panel, cx| panel.synchronize_context(target, table, cx));
        }
        if let Some(item) = self.active_item() {
            if let Some(index) = self
                .tabs
                .tabs()
                .iter()
                .position(|id| Some(*id) == self.tabs.active())
            {
                self.tab_scroll_handle.scroll_to_item(index);
            }
            item.focus(window, cx);
        } else {
            window.focus(&self.focus_handle, cx);
        }
        cx.notify();
    }

    fn has_active_modal(&self, cx: &App) -> bool {
        self.modal_layer.read(cx).has_active_modal()
    }

    fn new_query_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = cx.new(|cx| sql_language::editor(super::INITIAL_QUERY, window, cx));
        let target = self.connection_profiles.read(cx).query_target().cloned();
        let item = cx.new(|cx| {
            QueryItem::new(
                self.application.clone(),
                editor,
                self.settings.clone(),
                window,
                cx,
            )
        });
        let document_subscription =
            cx.subscribe(&item, |_, _, _: &QueryDocumentStateChanged, cx| cx.notify());
        if let Some(target) = target {
            item.update(cx, |item, cx| item.set_target(Some(target), window, cx));
        } else {
            item.update(cx, |item, cx| item.focus(window, cx));
        }
        let id = self.tabs.add();
        self.workspace_tabs.push(WorkspaceTab {
            id,
            key: WorkspaceItemKey::Query(id),
            item: WorkspaceItem::new(item),
            _subscriptions: vec![document_subscription],
        });
        self.focus_active_item(window, cx);
    }

    fn open_table_structure(
        &mut self,
        target: QueryTarget,
        table: TableRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = WorkspaceItemKey::TableStructure(target.clone(), table.clone());
        self.open_or_activate(key, window, cx, move |workspace, _, cx| {
            let item = cx.new(|cx| {
                TableStructureItem::new(
                    workspace.application.clone(),
                    target,
                    table,
                    workspace.settings.clone(),
                    cx,
                )
            });
            (WorkspaceItem::new(item), Vec::new())
        });
    }

    fn open_data_grid(
        &mut self,
        target: QueryTarget,
        table: TableRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = WorkspaceItemKey::DataGrid(target.clone(), table.clone());
        self.open_or_activate(key, window, cx, move |workspace, window, cx| {
            let item = cx.new(|cx| {
                DataGridItem::new(
                    workspace.application.clone(),
                    target,
                    table,
                    workspace.settings.clone(),
                    window,
                    cx,
                )
            });
            let observation = cx.observe(&item, |_, _, cx| cx.notify());
            (WorkspaceItem::new(item), vec![observation])
        });
    }

    fn open_object_definition(
        &mut self,
        object: ObjectDefinition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = WorkspaceItemKey::ObjectDefinition {
            target: object.target.clone(),
            kind: object.kind,
            name: object.name.clone(),
        };
        self.open_or_activate(key, window, cx, move |workspace, window, cx| {
            let item = cx.new(|cx| {
                ObjectDefinitionItem::new(object, workspace.settings.clone(), window, cx)
            });
            (WorkspaceItem::new(item), Vec::new())
        });
    }

    fn activate_tab(&mut self, id: WorkspaceTabId, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.activate(id) {
            self.focus_active_item(window, cx);
        }
    }

    fn close_tab(&mut self, id: WorkspaceTabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.workspace_tabs.iter().find(|tab| tab.id == id) else {
            return;
        };
        if tab.item.has_unsaved_changes(cx) {
            self.confirm_discard_and_close(id, window, cx);
            return;
        }
        self.close_tab_now(id, window, cx);
    }

    fn confirm_discard_and_close(
        &mut self,
        id: WorkspaceTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_prompt() {
            return;
        }
        let Some(tab) = self.workspace_tabs.iter().find(|tab| tab.id == id) else {
            return;
        };
        let language = self.settings.read(cx).language();
        let name = tab.item.discard_name(language, cx);
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

    fn close_tab_now(&mut self, id: WorkspaceTabId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.close(id) {
            return;
        }
        self.workspace_tabs.retain(|tab| tab.id != id);
        self.focus_active_item(window, cx);
    }

    fn next_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.next();
        self.focus_active_item(window, cx);
    }

    fn previous_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.previous();
        self.focus_active_item(window, cx);
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

    fn restart_application(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let unsaved_count = self
            .workspace_tabs
            .iter()
            .filter(|tab| tab.item.has_unsaved_changes(cx))
            .count();
        if unsaved_count == 0 {
            cx.restart();
            return;
        }
        if window.has_active_prompt() {
            return;
        }

        let language = self.settings.read(cx).language();
        let message = text(language, "重启 Astesia？", "Restart Astesia?");
        let unsaved_label = text(
            language,
            "重启将放弃未保存的标签页数量：",
            "Restarting will discard this many unsaved tabs:",
        );
        let detail = format!("{unsaved_label} {unsaved_count}");
        let answer = window.prompt(
            PromptLevel::Warning,
            message,
            Some(&detail),
            &[
                PromptButton::ok(text(language, "放弃并重启", "Discard and Restart")),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn_in(window, async move |workspace, cx| {
            if answer.await.ok() != Some(0) {
                return;
            }
            workspace.update_in(cx, |_, _, cx| cx.restart()).ok();
        })
        .detach();
    }

    fn execute_command(
        &mut self,
        command: WorkspaceCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            WorkspaceCommand::NewQuery => self.new_query_tab(window, cx),
            WorkspaceCommand::CloseActiveTab => self.close_active_tab(window, cx),
            WorkspaceCommand::NextTab => self.next_tab(window, cx),
            WorkspaceCommand::PreviousTab => self.previous_tab(window, cx),
            WorkspaceCommand::ToggleSidebar => self.toggle_sidebar(cx),
            WorkspaceCommand::RefreshConnections => {
                self.connection_profiles
                    .update(cx, |panel, cx| panel.refresh_profiles(cx));
            }
            WorkspaceCommand::ConnectProfile => self
                .connection_profiles
                .update(cx, |panel, cx| panel.connect_selected(window, cx)),
            WorkspaceCommand::DisconnectProfile => self
                .connection_profiles
                .update(cx, |panel, cx| panel.disconnect_selected(window, cx)),
            WorkspaceCommand::EditProfile => self
                .connection_profiles
                .update(cx, |panel, cx| panel.edit_selected(window, cx)),
            WorkspaceCommand::DeleteProfile => self
                .connection_profiles
                .update(cx, |panel, cx| panel.confirm_delete_selected(window, cx)),
            WorkspaceCommand::RestartApplication => self.restart_application(window, cx),
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
        let profile_actions = self.connection_profiles.read(cx).profile_actions();
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle_modal(window, cx, move |window, cx| {
                CommandPalette::new(language, profile_actions, window, cx)
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

    fn close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.tabs.active() {
            self.close_tab(id, window, cx);
        }
    }

    fn close_active_tab_action(
        &mut self,
        _: &CloseActiveQueryTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.has_active_modal(cx) {
            self.close_active_tab(window, cx);
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
        if !self.has_active_modal(cx)
            && !self
                .active_item()
                .is_some_and(|item| item.refresh_active_surface(cx))
        {
            self.connection_profiles
                .update(cx, |panel, cx| panel.refresh_profiles(cx));
        }
    }

    fn open_palette_click(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_command_palette(&ToggleCommandPalette, window, cx);
    }
}
