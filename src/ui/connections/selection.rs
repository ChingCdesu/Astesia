use super::*;

impl ConnectionProfilesPanel {
    pub(in crate::ui) fn select_query_context(
        &mut self,
        target: QueryTarget,
        databases: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if !self.state.query_target_is_live(&target) {
            self.state.clear_database_state(&target.connection_id);
            if let Some(request) = self.state.begin_database_load(&target.connection_id) {
                self.state.finish_database_load(
                    &request,
                    Ok(crate::application::LoadedDatabases {
                        session_generation: target.session_generation,
                        databases,
                    }),
                );
            }
        }
        self.synchronize_context(target, None, cx);
    }

    pub(in crate::ui) fn synchronize_context(
        &mut self,
        target: QueryTarget,
        table: Option<TableRef>,
        cx: &mut Context<Self>,
    ) {
        if !self.state.query_target_is_live(&target) {
            return;
        }
        self.selected_sidebar_row = Some(match table.as_ref() {
            Some(table) => format!(
                "table-{:?}",
                super::catalog_tree::CatalogTableKey::new(&target, table)
            ),
            None => format!("database-{target:?}"),
        });
        self.selected_profile_id = Some(target.connection_id.clone());
        self.selected_query_target = Some(target.clone());
        self.selected_catalog_table = table
            .as_ref()
            .map(|table| super::catalog_tree::CatalogTableKey::new(&target, table));
        self.expanded_databases.insert((
            target.connection_id.clone(),
            target.session_generation,
            target.database.clone(),
        ));
        self.load_objects(target, cx);
        self.notify_sidebar(cx);
    }
    pub(in crate::ui) fn profile_saved(&mut self, profile_id: String, cx: &mut Context<Self>) {
        self.selected_profile_id = Some(profile_id);
        self.refresh_profiles(cx);
    }

    pub(in crate::ui) fn status(&self, cx: &App) -> ConnectionProfilesStatus {
        derive_status(
            &self.state,
            self.selected_profile_id.as_deref(),
            self.selected_query_target.as_ref(),
            self.object_operation_in_progress,
            self.settings.read(cx).language(),
        )
    }

    pub(in crate::ui) fn query_target(&self) -> Option<&QueryTarget> {
        self.selected_query_target.as_ref()
    }

    pub(in crate::ui) fn profile_actions(&self) -> super::ProfileActions {
        let Some(profile) = self.selected_profile() else {
            return super::ProfileActions::default();
        };
        let busy = self.actions_blocked() || self.state.operation(&profile.profile.id).is_some();
        let editable = !busy
            && !profile
                .mcp_usage
                .as_ref()
                .is_some_and(|usage| usage.mcp_in_use);
        super::ProfileActions {
            connect: !busy && !profile.session.is_connected(),
            disconnect: editable && profile.session.is_connected(),
            edit: editable,
            delete: editable,
        }
    }

    pub(super) fn selected_profile(&self) -> Option<&ConnectionProfileSnapshot> {
        let selected = self.selected_profile_id.as_deref()?;
        self.state
            .snapshot()?
            .profiles
            .iter()
            .find(|profile| profile.profile.id == selected)
    }

    pub(super) fn actions_blocked(&self) -> bool {
        self.state.is_refreshing()
            || self.state.error().is_some()
            || self.object_operation_in_progress
    }

    pub(super) fn reconcile_selection(&mut self) {
        reconcile_selected_profile(&mut self.selected_profile_id, self.state.snapshot());
    }

    pub(super) fn reconcile_query_target(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.selected_query_target.as_ref() else {
            return;
        };
        if !self.state.query_target_is_live(target) {
            self.clear_query_target(cx);
        }
    }

    pub(super) fn toggle_database(&mut self, target: QueryTarget, cx: &mut Context<Self>) {
        self.selected_sidebar_row = Some(format!("database-{target:?}"));
        let key = (
            target.connection_id.clone(),
            target.session_generation,
            target.database.clone(),
        );
        if self.expanded_databases.remove(&key) {
            self.notify_sidebar(cx);
        } else {
            self.select_database(target, cx);
        }
    }

    pub(super) fn select_database(&mut self, target: QueryTarget, cx: &mut Context<Self>) {
        self.selected_sidebar_row = Some(format!("database-{target:?}"));
        self.selected_profile_id = Some(target.connection_id.clone());
        self.expanded_databases.insert((
            target.connection_id.clone(),
            target.session_generation,
            target.database.clone(),
        ));
        if self.selected_query_target.as_ref() != Some(&target) {
            self.selected_query_target = Some(target.clone());
            cx.emit(ConnectionProfilesEvent::QueryTargetSelected(target.clone()));
        }
        self.load_objects(target, cx);
        self.notify_sidebar(cx);
    }

    pub(super) fn request_table_structure(
        &mut self,
        target: QueryTarget,
        table: TableRef,
        cx: &mut Context<Self>,
    ) {
        if target.db_type.capabilities().sql && self.state.query_target_is_live(&target) {
            cx.emit(ConnectionProfilesEvent::TableStructureRequested { target, table });
        }
    }

    pub(super) fn request_primary_data(
        &mut self,
        target: QueryTarget,
        object: TableRef,
        cx: &mut Context<Self>,
    ) {
        if !self.state.query_target_is_live(&target) {
            return;
        }
        self.selected_profile_id = Some(target.connection_id.clone());
        self.selected_query_target = Some(target.clone());
        let key = super::catalog_tree::CatalogTableKey::new(&target, &object);
        self.selected_sidebar_row = Some(format!("table-{key:?}"));
        self.selected_catalog_table = Some(key);
        self.notify_sidebar(cx);
        match target.db_type {
            crate::db::DbType::MongoDB => {
                cx.emit(ConnectionProfilesEvent::DocumentCollectionRequested {
                    target,
                    collection: object,
                });
            }
            crate::db::DbType::Redis => {
                cx.emit(ConnectionProfilesEvent::RedisKeyRequested {
                    target,
                    key: object.name().to_string(),
                });
            }
            _ if target.db_type.capabilities().sql => {
                cx.emit(ConnectionProfilesEvent::TableDataRequested {
                    target,
                    table: object,
                });
            }
            _ => {}
        }
    }

    pub(super) fn request_object_definition(
        &mut self,
        object: ObjectDefinition,
        cx: &mut Context<Self>,
    ) {
        if object.target.db_type.capabilities().sql
            && self.state.query_target_is_live(&object.target)
        {
            cx.emit(ConnectionProfilesEvent::ObjectDefinitionRequested(object));
        }
    }

    pub(super) fn invalidate_query_session(&mut self, connection_id: &str, cx: &mut Context<Self>) {
        let selected_session_generation = self
            .selected_query_target
            .as_ref()
            .filter(|target| target.connection_id == connection_id)
            .map(|target| target.session_generation);
        let current_session_generation = self
            .state
            .snapshot()
            .and_then(|snapshot| {
                snapshot
                    .profiles
                    .iter()
                    .find(|profile| profile.profile.id == connection_id)
            })
            .and_then(|profile| profile.session.generation);
        if let Some(session_generation) = current_session_generation.or(selected_session_generation)
        {
            cx.emit(ConnectionProfilesEvent::QuerySessionInvalidated {
                connection_id: connection_id.to_string(),
                session_generation,
            });
        }
        if self
            .selected_query_target
            .as_ref()
            .is_some_and(|target| target.connection_id == connection_id)
        {
            self.selected_query_target = None;
            self.notify_sidebar(cx);
        }
    }

    fn clear_query_target(&mut self, cx: &mut Context<Self>) {
        if let Some(target) = self.selected_query_target.take() {
            cx.emit(ConnectionProfilesEvent::QueryTargetInvalidated(target));
            self.notify_sidebar(cx);
        }
    }

    pub(super) fn emit_query_sessions(&mut self, cx: &mut Context<Self>) {
        self.prune_catalog_details();
        if let Some(snapshot) = self.state.snapshot() {
            cx.emit(ConnectionProfilesEvent::QuerySessionsChanged(Arc::new(
                snapshot.clone(),
            )));
        }
    }

    pub(super) fn set_notice(&mut self, tone: NoticeTone, message: impl Into<String>) {
        self.notice = Some(PanelNotice {
            tone,
            message: message.into(),
        });
    }
}
pub(super) fn derive_status(
    state: &ConnectionWorkspaceState,
    selected_profile_id: Option<&str>,
    selected_target: Option<&QueryTarget>,
    object_operation_in_progress: bool,
    language: crate::platform::UiLanguage,
) -> ConnectionProfilesStatus {
    let context_profile = selected_target
        .map(|target| target.connection_id.as_str())
        .or(selected_profile_id);
    let selected = context_profile.and_then(|selected| {
        state
            .snapshot()?
            .profiles
            .iter()
            .find(|profile| profile.profile.id == selected)
    });
    let operation = selected.and_then(|profile| state.operation(&profile.profile.id));
    let summary = selected
        .map(|profile| {
            selected_target
                .filter(|target| target.connection_id == profile.profile.id)
                .map(|target| format!("{} / {}", profile.profile.name, target.database))
                .unwrap_or_else(|| profile.profile.name.clone())
        })
        .unwrap_or_else(|| text(language, "未连接", "Disconnected").to_string());
    let session = match (selected, operation) {
        (_, Some(ProfileOperationKind::Connecting)) => ConnectionSessionStatus::Connecting,
        (_, Some(ProfileOperationKind::Disconnecting)) => ConnectionSessionStatus::Disconnecting,
        (_, Some(ProfileOperationKind::Deleting)) => ConnectionSessionStatus::Deleting,
        (Some(profile), None) if profile.session.is_connected() => {
            ConnectionSessionStatus::Connected
        }
        (Some(_), None) => ConnectionSessionStatus::Disconnected,
        (None, _) if state.snapshot().is_some() => ConnectionSessionStatus::NoSelection,
        (None, _) => ConnectionSessionStatus::Loading,
    };
    let activity = if state.error().is_some() {
        ConnectionActivityStatus::NeedsRefresh
    } else if state.is_refreshing() {
        ConnectionActivityStatus::Refreshing
    } else if operation.is_some() || object_operation_in_progress {
        ConnectionActivityStatus::Working
    } else if selected.is_some_and(|profile| {
        matches!(
            state.databases(&profile.profile.id),
            Some(crate::application::connection_workspace::DatabaseListState::Loading { .. })
        )
    }) {
        ConnectionActivityStatus::LoadingDatabases
    } else if selected_target.is_some_and(|target| {
        state
            .objects(target)
            .is_some_and(crate::application::connection_workspace::ObjectListState::is_loading)
    }) {
        ConnectionActivityStatus::LoadingObjects
    } else if state.snapshot().is_some() {
        ConnectionActivityStatus::Ready
    } else {
        ConnectionActivityStatus::Loading
    };

    ConnectionProfilesStatus {
        summary,
        session,
        activity,
    }
}

pub(super) fn reconcile_selected_profile(
    selected_profile_id: &mut Option<String>,
    snapshot: Option<&crate::application::ConnectionWorkspaceSnapshot>,
) {
    let Some(selected) = selected_profile_id.as_ref() else {
        return;
    };
    let selection_exists = snapshot.is_some_and(|snapshot| {
        snapshot
            .profiles
            .iter()
            .any(|profile| &profile.profile.id == selected)
    });
    if !selection_exists {
        *selected_profile_id = None;
    }
}
