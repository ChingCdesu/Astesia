use super::*;

impl ConnectionProfilesPanel {
    pub(super) fn load_databases(&mut self, connection_id: String, cx: &mut Context<Self>) {
        let Some(request) = self.state.begin_database_load(&connection_id) else {
            return;
        };
        cx.notify();

        let application = self.application.clone();
        let language = self.settings.read(cx).language();
        let connection_id_for_task = connection_id.clone();
        let load = gpui_tokio::Tokio::spawn(cx, async move {
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
                        cx.notify();
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
        self.state.clear_database_state(&connection_id);
        self.load_databases(connection_id, cx);
    }

    pub(super) fn load_objects(&mut self, target: QueryTarget, cx: &mut Context<Self>) {
        let Some(requests) = self.state.begin_object_load(&target) else {
            return;
        };
        cx.notify();

        let language = self.settings.read(cx).language();
        for request in requests {
            let application = self.application.clone();
            let kind = request.kind();
            let connection_id = request.connection_id().to_string();
            let database = request.database().to_string();
            let load = gpui_tokio::Tokio::spawn(cx, async move {
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
                            cx.notify();
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
        self.state.clear_object_state(&target);
        self.load_objects(target, cx);
    }
}
