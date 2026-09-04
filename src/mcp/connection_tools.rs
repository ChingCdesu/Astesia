use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use serde_json::json;

use super::{
    execution::DATABASE_OPERATION_TIMEOUT, protocol::ConnectionIdArgs, session::PendingSyncLease,
    AstesiaMcp,
};

#[tool_router(router = connection_tools_router, vis = "pub(super)")]
impl AstesiaMcp {
    #[tool(
        description = "List desktop connection profiles that are enabled for MCP, without returning credentials.",
        annotations(
            title = "List Astesia connections",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_connections(&self) -> CallToolResult {
        let profiles = match self.catalog.profiles().await {
            Ok(profiles) => profiles,
            Err(error) => return Self::failure(error),
        };
        let mut output = Vec::with_capacity(profiles.len());
        for profile in profiles {
            let generation = self.catalog.connected_generation(&profile.config.id).await;
            output.push(json!({
                "connection_id": profile.config.id,
                "name": profile.config.name,
                "db_type": Self::db_type_name(&profile.config.db_type),
                "host": profile.config.host,
                "port": profile.config.port,
                "username": profile.config.username,
                "database": profile.config.database,
                "credential_source": profile.has_credential.then_some("system_vault"),
                "revision": profile.revision,
                "persistent": true,
                "connected": generation.is_some(),
                "generation": generation,
            }));
        }
        Self::success(json!(output))
    }

    #[tool(
        description = "Test a saved connection profile without keeping it open.",
        annotations(
            title = "Test Astesia connection",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn test_connection(
        &self,
        Parameters(args): Parameters<ConnectionIdArgs>,
    ) -> CallToolResult {
        let Some(sync) = self.sync.as_ref() else {
            let _lifecycle = self
                .catalog
                .lock_connection_lifecycle(&args.connection_id)
                .await;
            return match tokio::time::timeout(
                DATABASE_OPERATION_TIMEOUT,
                self.catalog.test_connection(&args.connection_id),
            )
            .await
            {
                Ok(Ok(())) => {
                    Self::success(json!({ "connection_id": args.connection_id, "reachable": true }))
                }
                Ok(Err(error)) => Self::failure(error),
                Err(_) => Self::failure("测试连接超时（60 秒）"),
            };
        };

        let lifecycle = self
            .catalog
            .lock_connection_lifecycle(&args.connection_id)
            .await;
        if self.active_tests.current(&args.connection_id).is_some() {
            return Self::failure(format!(
                "连接 {} 已有并发测试，请等待其完成后重试",
                args.connection_id
            ));
        }
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };
        let (generation, owns_sync_ownership, pending_sync_lease) =
            match self.catalog.connected_generation(&args.connection_id).await {
                Some(generation) => (generation, false, None),
                None => match sync
                    .acquire(args.connection_id.clone(), profile.revision)
                    .await
                {
                    Ok(generation) => (
                        generation,
                        true,
                        Some(PendingSyncLease::new(
                            sync.clone(),
                            args.connection_id.clone(),
                            generation,
                        )),
                    ),
                    Err(error) => {
                        return Self::failure(format!(
                            "无法向 Astesia App 申请测试连接占用: {error}"
                        ))
                    }
                },
            };
        let mut active_test = match self.active_tests.register(
            &args.connection_id,
            generation,
            owns_sync_ownership,
            pending_sync_lease,
        ) {
            Ok(active_test) => active_test,
            Err(error) => return Self::failure(error),
        };
        let prepared_test = match self
            .catalog
            .prepare_connection_test(&args.connection_id, profile.revision)
            .await
        {
            Ok(prepared_test) => prepared_test,
            Err(error) => {
                // prepare_connection_test has already dropped any partial OS
                // lease. Keep the same RAII/release path as a completed test.
                active_test.mark_future_dropped();
                // Let an already queued App control ACK this generation while
                // the best-effort Released request is in flight.
                drop(lifecycle);
                if owns_sync_ownership {
                    if let Err(release_error) = active_test.release_pending_sync_lease().await {
                        return Self::failure(format!(
                            "{error}; 同时无法释放 Astesia App 中的测试连接占用: {release_error}"
                        ));
                    }
                }
                return Self::failure(error);
            }
        };
        drop(lifecycle);

        let test_result = {
            let operation = tokio::time::timeout(DATABASE_OPERATION_TIMEOUT, prepared_test.run());
            tokio::pin!(operation);
            tokio::select! {
                result = &mut operation => Some(result),
                _ = active_test.cancellation.cancelled() => None,
            }
        };
        // The timeout/test future (and therefore its shared OS lease) is
        // dropped before controls are allowed to ACK this generation.
        active_test.mark_future_dropped();

        if owns_sync_ownership {
            if let Err(error) = active_test.release_pending_sync_lease().await {
                return Self::failure(format!(
                    "测试连接已结束，但无法释放 Astesia App 中的连接占用: {error}"
                ));
            }
        }

        match test_result {
            None => Self::failure("连接测试已由 Astesia App 取消"),
            Some(Ok(Ok(()))) => {
                Self::success(json!({ "connection_id": args.connection_id, "reachable": true }))
            }
            Some(Ok(Err(error))) => Self::failure(error),
            Some(Err(_)) => Self::failure("测试连接超时（60 秒）"),
        }
    }

    #[tool(
        description = "Open a desktop connection by ID for subsequent metadata, query, and row tools.",
        annotations(
            title = "Access Astesia connection",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn connect_connection(
        &self,
        Parameters(args): Parameters<ConnectionIdArgs>,
    ) -> CallToolResult {
        let _lifecycle = self
            .catalog
            .lock_connection_lifecycle(&args.connection_id)
            .await;
        if self.active_tests.current(&args.connection_id).is_some() {
            return Self::failure("连接测试正在进行或仍在释放占用，请稍后再连接");
        }
        let profile = match self.catalog.profile(&args.connection_id).await {
            Ok(profile) => profile,
            Err(error) => return Self::failure(error),
        };

        let mut pending_sync_lease = match self.sync.as_ref() {
            Some(sync) => match sync
                .acquire(args.connection_id.clone(), profile.revision)
                .await
            {
                Ok(generation) => Some(
                    PendingSyncLease::new(sync.clone(), args.connection_id.clone(), generation)
                        .for_connection(self.catalog.clone()),
                ),
                Err(error) => {
                    return Self::failure(format!("无法向 Astesia App 申请连接占用: {error}"))
                }
            },
            None => None,
        };
        let acquired_generation = pending_sync_lease
            .as_ref()
            .map(PendingSyncLease::generation);

        let connect_result = if let Some(generation) = acquired_generation {
            tokio::time::timeout(
                DATABASE_OPERATION_TIMEOUT,
                self.catalog.connect_with_generation(
                    &args.connection_id,
                    generation,
                    profile.revision,
                ),
            )
            .await
        } else {
            tokio::time::timeout(
                DATABASE_OPERATION_TIMEOUT,
                self.catalog.connect(&args.connection_id),
            )
            .await
        };

        let outcome = match connect_result {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(error)) => {
                if let Some(pending_sync_lease) = pending_sync_lease.as_mut() {
                    if let Err(release_error) = pending_sync_lease.release().await {
                        return Self::failure(format!(
                            "{error}; 同时无法释放 Astesia App 中的连接占用: {release_error}"
                        ));
                    }
                }
                return Self::failure(error);
            }
            Err(_) => {
                if let Some(pending_sync_lease) = pending_sync_lease.as_mut() {
                    if let Err(release_error) = pending_sync_lease.release().await {
                        return Self::failure(format!(
                            "访问连接超时（60 秒）；同时无法释放 Astesia App 中的连接占用: {release_error}"
                        ));
                    }
                }
                return Self::failure("访问连接超时（60 秒）");
            }
        };

        if let Some(pending_sync_lease) = pending_sync_lease.as_mut() {
            if let Err(error) = pending_sync_lease.commit_connected().await {
                let local_rollback = self.catalog.disconnect(&args.connection_id).await.result;
                let app_rollback = pending_sync_lease.release().await;
                let rollback_status = match (local_rollback, app_rollback) {
                    (Ok(_), Ok(_)) => "连接访问已完整回滚".to_string(),
                    (local, app) => format!(
                        "连接访问回滚不完整（MCP: {}; App: {}）",
                        local
                            .err()
                            .map(|error| error.to_string())
                            .unwrap_or_else(|| "已断开".to_string()),
                        app.err()
                            .map(|error| error.to_string())
                            .unwrap_or_else(|| "已释放".to_string()),
                    ),
                };
                return Self::failure(format!(
                    "无法同步连接状态到 Astesia App: {error}; {rollback_status}"
                ));
            }
        }

        Self::success(json!({
            "connection_id": args.connection_id,
            "connected": true,
            "opened_now": outcome.opened_now,
            "generation": outcome.generation,
            "app_synced": self.sync.is_some(),
        }))
    }

    #[tool(
        description = "Close the active MCP database connection while retaining its desktop profile.",
        annotations(
            title = "Disconnect Astesia connection",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn disconnect_connection(
        &self,
        Parameters(args): Parameters<ConnectionIdArgs>,
    ) -> CallToolResult {
        let _lifecycle = self
            .catalog
            .lock_connection_lifecycle(&args.connection_id)
            .await;
        let active_test = self.active_tests.current(&args.connection_id);
        if let Some(active_test) = active_test.as_ref() {
            active_test.cancel();
            active_test.wait_until_future_dropped().await;
        }
        let disconnect_outcome = self.catalog.disconnect(&args.connection_id).await;
        let generation = active_test
            .as_ref()
            .map(|active_test| active_test.generation)
            .or(disconnect_outcome.generation);
        let disconnected = disconnect_outcome
            .result
            .map(|closed| closed || active_test.is_some());
        let sync_error = match (self.sync.as_ref(), generation) {
            (Some(sync), Some(generation)) => sync
                .released(args.connection_id.clone(), generation)
                .await
                .err()
                .map(|error| error.to_string()),
            _ => None,
        };

        match (disconnected, sync_error) {
            (Ok(closed), None) => Self::success(json!({
                "connection_id": args.connection_id,
                "connected": false,
                "closed_now": closed,
                "generation": generation,
                "app_synced": self.sync.is_some(),
            })),
            (Ok(_), Some(error)) => {
                Self::failure(format!("连接已断开，但无法同步到 Astesia App: {error}"))
            }
            (Err(error), Some(sync_error)) => {
                Self::failure(format!("{error}; 同时无法同步到 Astesia App: {sync_error}"))
            }
            (Err(error), None) => Self::failure(error),
        }
    }
}
