use super::*;
use crate::ui::components::{ContextMenu, ContextMenuEntry};

#[derive(Clone)]
enum ContextChoice {
    Database(QueryTarget, Arc<Vec<String>>),
    Schema(Option<String>),
    Message(String),
}

impl QueryItem {
    pub(super) fn render_context_picker(
        &self,
        target_label: String,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let language = self.settings.read(cx).language();
        let blocked =
            self.state.is_running() || self.file_operation_busy() || self.context_picker_busy;
        h_flex()
            .gap_1()
            .min_w_0()
            .child(
                Button::new("query-database", "")
                    .size(ButtonSize::Compact)
                    .aria_label(text(language, "选择数据库", "Select database"))
                    .child(Label::new(target_label).text_size(px(10.0)).truncate())
                    .end_icon(
                        Icon::new(IconName::ChevronDown)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .loading(self.context_picker_busy)
                    .disabled(blocked)
                    .on_click(cx.listener(|item, event: &ClickEvent, window, cx| {
                        item.open_context_picker(false, event.position(), window, cx)
                    })),
            )
            .when(
                self.state
                    .target()
                    .is_none_or(|target| target.db_type.supports_query_schema()),
                |row| {
                    row.child(
                        Button::new("query-schema", "")
                            .size(ButtonSize::Compact)
                            .aria_label(text(language, "选择 Schema", "Select schema"))
                            .child(
                                Label::new(self.selected_schema.clone().unwrap_or_else(|| {
                                    text(language, "默认 Schema", "Default schema").into()
                                }))
                                .text_size(px(10.0)),
                            )
                            .end_icon(
                                Icon::new(IconName::ChevronDown)
                                    .size(IconSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .disabled(blocked || self.state.target().is_none())
                            .on_click(cx.listener(|item, event: &ClickEvent, window, cx| {
                                item.open_context_picker(true, event.position(), window, cx)
                            })),
                    )
                },
            )
    }

    fn open_context_picker(
        &mut self,
        schemas: bool,
        position: gpui_kit::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.is_running() || self.file_operation_busy() || self.context_picker_busy {
            return;
        }
        let previous_focus = window.focused(cx);
        let target = self.state.target().cloned();
        if schemas
            && !target
                .as_ref()
                .is_some_and(|target| target.db_type.supports_query_schema())
        {
            return;
        }
        self.context_menu = None;
        self.context_picker_busy = true;
        self.context_picker_generation = self.context_picker_generation.wrapping_add(1);
        let generation = self.context_picker_generation;
        let application = self.application.clone();
        let load = crate::ui::runtime::spawn(cx, async move {
            if schemas {
                let target = target.expect("schema picker requires a target");
                let mut choices = vec![ContextChoice::Schema(None)];
                choices.extend(
                    application
                        .catalog()
                        .schemas(&target.connection_id, &target.database)
                        .await?
                        .into_iter()
                        .map(|schema| ContextChoice::Schema(Some(schema))),
                );
                return Ok::<_, String>(choices);
            }
            let snapshot = application
                .connections()
                .snapshot()
                .await
                .map_err(|error| error.to_string())?;
            let mut choices = Vec::new();
            for profile in snapshot.profiles {
                let Some(session_generation) = profile.session.generation else {
                    continue;
                };
                if !profile.profile.db_type.capabilities().sql {
                    continue;
                }
                match application.catalog().databases(&profile.profile.id).await {
                    Ok(databases) => {
                        let databases = Arc::new(databases);
                        for database in databases.iter() {
                            choices.push(ContextChoice::Database(
                                QueryTarget {
                                    connection_id: profile.profile.id.clone(),
                                    connection_name: profile.profile.name.clone(),
                                    database: database.clone(),
                                    db_type: profile.profile.db_type,
                                    session_generation,
                                },
                                databases.clone(),
                            ));
                        }
                    }
                    Err(error) => choices.push(ContextChoice::Message(format!(
                        "{}: {error}",
                        profile.profile.name
                    ))),
                }
            }
            Ok(choices)
        });
        cx.spawn(async move |item, cx| {
            let result = load.await.unwrap_or_else(|error| Err(error.to_string()));
            item.update_in(cx, |item, window, cx| {
                if generation != item.context_picker_generation {
                    return;
                }
                item.context_picker_busy = false;
                if item.state.is_running()
                    || previous_focus
                        .as_ref()
                        .is_some_and(|focus| !focus.contains_focused(window, cx))
                {
                    cx.notify();
                    return;
                }
                let choices = result.unwrap_or_else(|error| vec![ContextChoice::Message(error)]);
                item.show_context_choices(choices, position, window, cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn select_query_schema(
        &mut self,
        schema: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.is_running()
            || !self
                .state
                .target()
                .is_some_and(|target| target.db_type.supports_query_schema())
        {
            return;
        }
        if self.selected_schema != schema {
            self.selected_schema = schema.clone();
            self.completion.set_schema(schema);
            self.state.clear_results();
            self.chart = None;
            self.showing_chart = false;
            if let Some(editor) = self.editor.read(cx).code_state().cloned() {
                editor.update(cx, |editor, cx| editor.dismiss_completion_overlay(cx));
            }
        }
        window.focus(&self.editor.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    fn show_context_choices(
        &mut self,
        choices: Vec<ContextChoice>,
        position: gpui_kit::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let language = self.settings.read(cx).language();
        let selected_target = self.state.target().cloned();
        let selected_schema = self.selected_schema.clone();
        let owner = cx.entity().downgrade();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            if choices.is_empty() {
                return menu.label(text(language, "请先连接数据库", "Connect a database first"));
            }
            for choice in choices {
                let (label, checked) = match &choice {
                    ContextChoice::Database(target, _) => (
                        format!("{} / {}", target.connection_name, target.database),
                        selected_target.as_ref() == Some(target),
                    ),
                    ContextChoice::Schema(schema) => (
                        schema.clone().unwrap_or_else(|| {
                            text(language, "使用数据库默认值", "Use database default").into()
                        }),
                        selected_schema == *schema,
                    ),
                    ContextChoice::Message(message) => {
                        menu = menu.label(message.clone());
                        continue;
                    }
                };
                let owner = owner.clone();
                menu = menu.item(ContextMenuEntry::new(label).checked(checked).on_click(
                    move |_, window, cx| {
                        owner
                            .update(cx, |item, cx| {
                                if item.state.is_running() || item.file_operation_busy() {
                                    return;
                                }
                                match &choice {
                                    ContextChoice::Database(target, databases) => {
                                        item.set_target(Some(target.clone()), window, cx);
                                        cx.emit(QueryContextChanged {
                                            target: target.clone(),
                                            databases: databases.as_ref().clone(),
                                        });
                                    }
                                    ContextChoice::Schema(schema) => {
                                        item.select_query_schema(schema.clone(), window, cx);
                                    }
                                    ContextChoice::Message(_) => {}
                                }
                            })
                            .ok();
                    },
                ));
            }
            menu
        });
        let previous_focus = window.focused(cx);
        window.focus(&menu.focus_handle(cx), cx);
        let subscription = cx.subscribe_in(
            &menu,
            window,
            move |item, menu, _: &gpui_kit::DismissEvent, window, cx| {
                if menu.focus_handle(cx).contains_focused(window, cx) {
                    if let Some(focus) = &previous_focus {
                        window.focus(focus, cx);
                    }
                }
                item.context_menu = None;
                cx.notify();
            },
        );
        self.context_menu = Some((menu, position, subscription));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        connection_repository::SharedConnectionRepository,
        credential_vault::test_support::MemoryCredentialVault, platform::DesktopPreferences,
        ui::sql_language,
    };

    #[gpui_kit::test]
    fn context_menus_select_database_and_schema_without_editing_sql(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(|cx| {
            crate::ui::initialize_editor_runtime(crate::platform::ThemePreference::Light, cx)
        });
        let directory = tempfile::tempdir().unwrap();
        let application = Arc::new(Application::with_repository(
            SharedConnectionRepository::new(
                directory.path().join("connections.sqlite3"),
                MemoryCredentialVault::shared(),
            ),
        ));
        let settings = cx.new(|_| ShellSettings::new(DesktopPreferences::default(), None));
        let window = cx.add_window(|window, cx| {
            let editor = cx.new(|cx| sql_language::editor("SELECT * FROM users;", window, cx));
            QueryItem::new(application, editor, settings, window, cx)
        });
        let item = window.root(cx).unwrap();
        let target = QueryTarget {
            connection_id: "test".into(),
            connection_name: "Test".into(),
            database: "analytics".into(),
            db_type: DbType::PostgreSQL,
            session_generation: 1,
        };
        window
            .update(cx, |item, window, cx| {
                item.show_context_choices(
                    vec![ContextChoice::Database(
                        target.clone(),
                        Arc::new(vec!["analytics".into()]),
                    )],
                    point(px(10.0), px(10.0)),
                    window,
                    cx,
                )
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "down enter");
        cx.run_until_parked();
        assert_eq!(
            item.read_with(cx, |item, _| item.state.target().cloned()),
            Some(target)
        );
        window
            .update(cx, |item, window, cx| {
                item.show_context_choices(
                    vec![ContextChoice::Schema(Some("reporting".into()))],
                    point(px(10.0), px(10.0)),
                    window,
                    cx,
                )
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "down enter");
        cx.run_until_parked();
        assert_eq!(
            item.read_with(cx, |item, _| item.selected_schema.clone()),
            Some("reporting".into())
        );
        assert_eq!(
            item.read_with(cx, |item, cx| item.editor.read(cx).text(cx)),
            "SELECT * FROM users;"
        );
        window
            .update(cx, |item, window, cx| {
                item.state
                    .begin_execution(
                        QueryDocument::new("SELECT 1".into(), 0..0),
                        QueryExecutionScope::All,
                    )
                    .unwrap();
                item.select_query_schema(Some("other_schema".into()), window, cx);
                assert_eq!(item.selected_schema.as_deref(), Some("reporting"));
                let mut target = item.state.target().unwrap().clone();
                target.database = "other".into();
                item.set_target(Some(target), window, cx);
                assert!(item.selected_schema.is_none());
            })
            .unwrap();
    }
}
