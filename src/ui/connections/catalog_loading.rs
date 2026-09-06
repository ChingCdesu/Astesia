use super::*;

impl ConnectionProfilesPanel {
    pub(super) fn load_databases(&mut self, connection_id: String, cx: &mut Context<Self>) {
        let Some(request) = self.state.begin_database_load(&connection_id) else {
            return;
        };
        self.notify_sidebar(cx);

        let application = self.application.clone();
        let language = self.settings.read(cx).language();
        let connection_id_for_task = connection_id.clone();
        let load = crate::ui::runtime::spawn(cx, async move {
            application
                .connections()
                .load_databases(&connection_id_for_task)
                .await
        });
        cx.spawn(async move |panel, cx| {
            let result = match load.await {
                Ok(result) => result,
                Err(error) => Err(format!(
                    "{}: {error}",
                    text(
                        language,
                        "数据库列表后台任务意外结束",
                        "The database-list task ended unexpectedly",
                    )
                )),
            };
            panel
                .update(cx, |panel, cx| {
                    if panel.state.finish_database_load(&request, result) {
                        panel.reconcile_query_target(cx);
                        if let Some(profile) = panel
                            .state
                            .snapshot()
                            .and_then(|snapshot| {
                                snapshot
                                    .profiles
                                    .iter()
                                    .find(|profile| profile.profile.id == connection_id)
                            })
                            .cloned()
                        {
                            let targets = panel
                                .expanded_databases
                                .iter()
                                .filter(|(id, _, _)| id == &connection_id)
                                .map(|(id, generation, database)| QueryTarget {
                                    connection_id: id.clone(),
                                    connection_name: profile.profile.name.clone(),
                                    database: database.clone(),
                                    db_type: profile.profile.db_type,
                                    session_generation: *generation,
                                })
                                .collect::<Vec<_>>();
                            for target in targets {
                                panel.refresh_table_details(&target, cx);
                                panel.load_objects(target, cx);
                            }
                        }
                        panel.notify_sidebar(cx);
                    }
                })
                .ok();
        })
        .detach();
    }

    pub(super) fn retry_databases(
        &mut self,
        connection_id: String,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_profile_databases(connection_id, cx);
    }

    pub(super) fn refresh_profile_databases(
        &mut self,
        connection_id: String,
        cx: &mut Context<Self>,
    ) {
        let application = self.application.clone();
        let id = connection_id.clone();
        let refresh = crate::ui::runtime::spawn(cx, async move {
            application.catalog().refresh_schema(&id, None).await
        });
        cx.spawn(async move |panel, cx| {
            let result = refresh.await.unwrap_or_else(|error| Err(error.to_string()));
            panel
                .update(cx, |panel, cx| {
                    if let Err(error) = result {
                        panel.notice = Some(PanelNotice {
                            tone: NoticeTone::Error,
                            message: error,
                        });
                        panel.notify_sidebar(cx);
                        return;
                    }
                    panel
                        .application
                        .query_completions()
                        .invalidate_connection(&connection_id);
                    panel.state.clear_database_state(&connection_id);
                    panel
                        .table_details
                        .retain(|key, _| !key.belongs_to_connection(&connection_id));
                    panel.load_databases(connection_id, cx);
                })
                .ok();
        })
        .detach();
    }

    pub(super) fn load_objects(&mut self, target: QueryTarget, cx: &mut Context<Self>) {
        let Some(requests) = self.state.begin_object_load(&target) else {
            return;
        };
        self.notify_sidebar(cx);

        let language = self.settings.read(cx).language();
        for request in requests {
            let application = self.application.clone();
            let kind = request.kind();
            let connection_id = request.connection_id().to_string();
            let database = request.database().to_string();
            let load = crate::ui::runtime::spawn(cx, async move {
                application
                    .catalog()
                    .catalog_section(&connection_id, &database, kind)
                    .await
            });
            cx.spawn(async move |panel, cx| {
                let result = match load.await {
                    Ok(result) => result,
                    Err(error) => crate::application::CatalogEntry::failed(
                        kind,
                        format!(
                            "{}: {error}",
                            text(
                                language,
                                "数据库对象后台任务意外结束",
                                "The database-object task ended unexpectedly",
                            )
                        ),
                    ),
                };
                panel
                    .update(cx, |panel, cx| {
                        if panel.state.finish_object_load(&request, result) {
                            panel.notify_sidebar(cx);
                        }
                    })
                    .ok();
            })
            .detach();
        }
    }

    pub(super) fn retry_objects(
        &mut self,
        target: QueryTarget,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_target_objects(target, cx);
    }
}
