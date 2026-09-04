use super::presentation::repository_error_message;
use super::*;

impl ConnectionProfilesPanel {
    pub(super) fn refresh(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh_profiles(cx);
    }

    pub(in crate::ui) fn refresh_profiles(&mut self, cx: &mut Context<Self>) {
        let request = self.state.begin_refresh();
        cx.notify();

        let application = self.application.clone();
        let load =
            gpui_tokio::Tokio::spawn(
                cx,
                async move { application.connections().snapshot().await },
            );
        cx.spawn(async move |panel, cx| {
            let result = match load.await {
                Ok(snapshot) => snapshot.map_err(ConnectionWorkspaceError::from),
                Err(error) => Err(ConnectionWorkspaceError::load_profiles(error)),
            };
            panel
                .update(cx, |panel, cx| {
                    let applied = panel.state.finish_refresh(request, result);
                    if applied == SnapshotApply::Applied {
                        panel.reconcile_selection();
                        panel.reconcile_query_target(cx);
                        panel.emit_query_sessions(cx);
                    }
                    if applied != SnapshotApply::Superseded {
                        cx.notify();
                    }
                })
                .ok();
        })
        .detach();
    }

    pub(super) fn select_profile(
        &mut self,
        profile_id: String,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_profile_id(profile_id, cx);
    }

    pub(super) fn select_profile_id(&mut self, profile_id: String, cx: &mut Context<Self>) {
        let changed = self.selected_profile_id.as_deref() != Some(&profile_id);
        self.selected_profile_id = Some(profile_id.clone());
        if changed {
            cx.notify();
        }
        self.load_databases(profile_id, cx);
    }

    pub(super) fn create_profile(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.actions_blocked() {
            return;
        }
        cx.emit(ConnectionProfilesEvent::CreateRequested);
    }

    pub(super) fn edit_selected(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.actions_blocked() {
            return;
        }
        let Some(selected) = self.selected_profile() else {
            return;
        };
        if selected
            .mcp_usage
            .as_ref()
            .is_some_and(|usage| usage.mcp_in_use)
            || self.state.operation(&selected.profile.id).is_some()
        {
            return;
        }
        cx.emit(ConnectionProfilesEvent::EditRequested(Arc::new(
            selected.profile.clone(),
        )));
    }

    pub(super) fn connect_selected(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = self
            .selected_profile()
            .filter(|profile| !profile.session.is_connected())
            .map(|profile| profile.profile.id.clone())
        else {
            return;
        };
        self.connect(connection_id, cx);
    }

    fn connect(&mut self, connection_id: String, cx: &mut Context<Self>) {
        if self.actions_blocked() {
            return;
        }
        let Some(request) = self
            .state
            .begin_operation(&connection_id, ProfileOperationKind::Connecting)
        else {
            return;
        };
        self.notice = None;
        cx.notify();
        self.run_profile_operation(
            request,
            ProfileOperationCommand::Connect { connection_id },
            cx,
        );
    }

    pub(super) fn disconnect_selected(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.actions_blocked() {
            return;
        }
        let Some(connection_id) = self
            .selected_profile()
            .filter(|profile| profile.session.is_connected())
            .map(|profile| profile.profile.id.clone())
        else {
            return;
        };
        let Some(request) = self
            .state
            .begin_operation(&connection_id, ProfileOperationKind::Disconnecting)
        else {
            return;
        };
        self.state.clear_database_state(&connection_id);
        self.invalidate_query_session(&connection_id, cx);
        self.notice = None;
        cx.notify();
        self.run_profile_operation(
            request,
            ProfileOperationCommand::Disconnect { connection_id },
            cx,
        );
    }

    fn run_profile_operation(
        &mut self,
        request: ProfileOperationRequest,
        command: ProfileOperationCommand,
        cx: &mut Context<Self>,
    ) {
        let connection_id = command.connection_id().to_string();
        let operation_name = command.operation_name();
        let language = self.settings.read(cx).language();
        let unexpected_message = match &command {
            ProfileOperationCommand::Connect { .. } => text(
                language,
                "连接后台任务意外结束；请刷新后确认当前连接状态。",
                "The connect task ended unexpectedly. Refresh to confirm the current state.",
            ),
            ProfileOperationCommand::Disconnect { .. } => text(
                language,
                "断开后台任务意外结束；请刷新后确认当前连接状态。",
                "The disconnect task ended unexpectedly. Refresh to confirm the current state.",
            ),
            ProfileOperationCommand::Delete { .. } => text(
                language,
                "删除后台任务意外结束；请刷新后确认连接配置是否仍存在。",
                "The delete task ended unexpectedly. Refresh to confirm whether the profile still exists.",
            ),
        };
        let application = self.application.clone();
        let operation = gpui_tokio::Tokio::spawn(cx, async move {
            application
                .connections()
                .perform_profile_operation(command)
                .await
        });

        cx.spawn(async move |panel, cx| {
            let result = operation.await;
            panel
                .update(cx, |panel, cx| match result {
                    Ok(completion) => {
                        panel.apply_profile_operation(&request, completion, &connection_id, cx)
                    }
                    Err(error) => {
                        if panel.state.finish_operation(
                            &request,
                            Err(ConnectionWorkspaceError::operation(operation_name, error)),
                        ) != OperationApply::Discarded
                        {
                            panel.set_notice(NoticeTone::Error, unexpected_message);
                            cx.notify();
                        }
                    }
                })
                .ok();
        })
        .detach();
    }

    fn apply_profile_operation(
        &mut self,
        request: &ProfileOperationRequest,
        completion: ProfileOperationCompletion,
        connection_id: &str,
        cx: &mut Context<Self>,
    ) {
        let apply = self.state.finish_operation(
            request,
            completion.snapshot.map_err(ConnectionWorkspaceError::from),
        );
        let language = self.settings.read(cx).language();
        match apply {
            OperationApply::Discarded => return,
            OperationApply::Snapshot(SnapshotApply::Applied) => {
                self.reconcile_selection();
                self.reconcile_query_target(cx);
                self.emit_query_sessions(cx);
                if self.apply_operation_outcome(completion.outcome, language) {
                    self.load_databases(connection_id.to_string(), cx);
                }
            }
            OperationApply::Snapshot(SnapshotApply::Failed) => {
                self.set_notice(
                    NoticeTone::Error,
                    operation_snapshot_failure_message(&completion.outcome, language),
                );
            }
            OperationApply::Snapshot(SnapshotApply::Superseded) => {
                self.apply_superseded_outcome(completion.outcome, language);
                self.refresh_profiles(cx);
            }
        }
        cx.notify();
    }

    fn apply_operation_outcome(
        &mut self,
        outcome: ProfileOperationOutcome,
        language: crate::platform::UiLanguage,
    ) -> bool {
        match outcome {
            ProfileOperationOutcome::Connected(Ok(ConnectionOutcome::Succeeded)) => {
                self.set_notice(
                    NoticeTone::Info,
                    text(language, "连接成功", "Connected successfully"),
                );
                true
            }
            ProfileOperationOutcome::Connected(Ok(ConnectionOutcome::Rejected(message)))
            | ProfileOperationOutcome::Connected(Err(message)) => {
                self.set_notice(NoticeTone::Error, message);
                false
            }
            ProfileOperationOutcome::Disconnected(report) => {
                let tone = if report.is_complete() {
                    NoticeTone::Info
                } else {
                    NoticeTone::Warning
                };
                self.set_notice(tone, report.message());
                false
            }
            ProfileOperationOutcome::Deleted(Ok(deleted)) => {
                let message = if deleted.credential_cleanup_pending {
                    text(
                        language,
                        "连接配置已删除；系统凭据将在稍后完成清理",
                        "Connection profile deleted; system credentials will be cleaned up later",
                    )
                } else {
                    text(language, "连接配置已删除", "Connection profile deleted")
                };
                self.set_notice(NoticeTone::Info, message);
                false
            }
            ProfileOperationOutcome::Deleted(Err(error)) => {
                self.set_notice(NoticeTone::Error, repository_error_message(&error));
                false
            }
        }
    }

    fn apply_superseded_outcome(
        &mut self,
        outcome: ProfileOperationOutcome,
        language: crate::platform::UiLanguage,
    ) {
        match outcome {
            ProfileOperationOutcome::Connected(_) => self.set_notice(
                NoticeTone::Warning,
                text(
                    language,
                    "连接结果已返回，正在重新同步最新状态…",
                    "The connect result arrived; resyncing the latest state…",
                ),
            ),
            ProfileOperationOutcome::Disconnected(_) => self.set_notice(
                NoticeTone::Warning,
                text(
                    language,
                    "断开结果已返回，正在重新同步最新状态…",
                    "The disconnect result arrived; resyncing the latest state…",
                ),
            ),
            ProfileOperationOutcome::Deleted(Ok(_)) => self.set_notice(
                NoticeTone::Warning,
                text(
                    language,
                    "连接配置已删除，正在重新同步最新列表…",
                    "The connection profile was deleted; resyncing the latest list…",
                ),
            ),
            ProfileOperationOutcome::Deleted(Err(error)) => {
                self.set_notice(NoticeTone::Error, repository_error_message(&error))
            }
        }
    }

    pub(super) fn confirm_delete_selected(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.actions_blocked() {
            return;
        }
        let Some(selected) = self.selected_profile() else {
            return;
        };
        if selected
            .mcp_usage
            .as_ref()
            .is_some_and(|usage| usage.mcp_in_use)
            || self.state.operation(&selected.profile.id).is_some()
        {
            return;
        }
        let connection_id = selected.profile.id.clone();
        let expected_revision = selected.profile.revision;
        let language = self.settings.read(cx).language();
        let message = format!(
            "{} “{}”?",
            text(language, "删除连接配置", "Delete connection profile"),
            selected.profile.name
        );
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            Some(text(
                language,
                "保存的凭据也会被删除。此操作无法撤销。",
                "Stored credentials will also be deleted. This cannot be undone.",
            )),
            &[
                PromptButton::ok(text(language, "删除", "Delete")),
                PromptButton::cancel(text(language, "取消", "Cancel")),
            ],
            cx,
        );
        cx.spawn(async move |panel, cx| {
            if answer.await.ok() != Some(0) {
                return;
            }
            panel
                .update(cx, |panel, cx| {
                    panel.delete_profile(connection_id, expected_revision, cx);
                })
                .ok();
        })
        .detach();
    }

    fn delete_profile(
        &mut self,
        connection_id: String,
        expected_revision: i64,
        cx: &mut Context<Self>,
    ) {
        if self.actions_blocked() {
            return;
        }
        let Some(request) = self
            .state
            .begin_operation(&connection_id, ProfileOperationKind::Deleting)
        else {
            return;
        };
        self.invalidate_query_session(&connection_id, cx);
        self.notice = None;
        cx.notify();
        self.run_profile_operation(
            request,
            ProfileOperationCommand::Delete {
                connection_id,
                expected_revision,
            },
            cx,
        );
    }
}

fn operation_snapshot_failure_message(
    outcome: &ProfileOperationOutcome,
    language: crate::platform::UiLanguage,
) -> &'static str {
    match outcome {
        ProfileOperationOutcome::Connected(_) => text(
            language,
            "连接结果已返回，但无法刷新连接状态；请刷新后再继续。",
            "The connect result arrived, but the connection state could not be refreshed. Refresh before continuing.",
        ),
        ProfileOperationOutcome::Disconnected(_) => text(
            language,
            "断开结果已返回，但无法刷新连接状态；请刷新后再继续。",
            "The disconnect result arrived, but the connection state could not be refreshed. Refresh before continuing.",
        ),
        ProfileOperationOutcome::Deleted(Ok(_)) => text(
            language,
            "连接配置已删除，但无法刷新列表；当前列表已锁定，请刷新后再继续。",
            "The profile was deleted, but the list could not be refreshed. Refresh before continuing.",
        ),
        ProfileOperationOutcome::Deleted(Err(_)) => text(
            language,
            "删除失败，且无法刷新连接列表；请刷新后再继续。",
            "Deletion failed and the connection list could not be refreshed. Refresh before continuing.",
        ),
    }
}
