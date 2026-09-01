use std::sync::{Arc, Weak};

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::mcp_sync::{McpSyncRequest, McpSyncResponse};

use super::{
    protocol::{failure, request_context, validate_context},
    state::OperationKey,
    McpSyncRegistry, MAX_RETAINED_KEYS,
};

impl McpSyncRegistry {
    #[cfg(test)]
    pub(crate) async fn apply_test_request(
        &self,
        expected_service_id: Uuid,
        request: McpSyncRequest,
    ) -> McpSyncResponse {
        self.apply(expected_service_id, request).await
    }

    pub(in crate::mcp_sync_server) async fn apply(
        &self,
        expected_service_id: Uuid,
        request: McpSyncRequest,
    ) -> McpSyncResponse {
        let context = request_context(&request);
        if let Err(error) = validate_context(context, expected_service_id) {
            return failure(error);
        }

        let operation_key = OperationKey {
            service_id: context.service_id,
            session_id: context.session_id,
            operation_id: context.operation_id,
        };
        let operation_lock = self.operation_lock(operation_key).await;
        let _operation_guard = operation_lock.lock().await;
        if let Some(response) = self.completed_response(operation_key).await {
            return response;
        }

        let response = match request {
            McpSyncRequest::Acquire {
                context,
                connection_id,
                profile_revision,
            } => self.acquire(context, connection_id, profile_revision).await,
            McpSyncRequest::Connected {
                context,
                connection_id,
                generation,
            } => self.connected(context, connection_id, generation).await,
            McpSyncRequest::Released {
                context,
                connection_id,
                generation,
            } => self.released(context, connection_id, generation).await,
            McpSyncRequest::PollControl { context } => self.poll_control(context).await,
            McpSyncRequest::ControlResult {
                context,
                command_id,
                connection_id,
                generation,
                ok,
                error,
            } => {
                self.control_result(context, command_id, connection_id, generation, ok, error)
                    .await
            }
            McpSyncRequest::SessionClosed { context } => self.close_session(context).await,
        };

        self.remember_response(operation_key, response.clone())
            .await;
        response
    }

    async fn operation_lock(&self, key: OperationKey) -> Arc<Mutex<()>> {
        let mut locks = self.operation_locks.lock().await;
        if locks.len() >= MAX_RETAINED_KEYS {
            locks.retain(|_, lock| lock.strong_count() > 0);
        }
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }

    async fn completed_response(&self, key: OperationKey) -> Option<McpSyncResponse> {
        self.inner
            .lock()
            .await
            .completed_operations
            .get(&key)
            .cloned()
    }

    async fn remember_response(&self, key: OperationKey, response: McpSyncResponse) {
        let mut state = self.inner.lock().await;
        if state.completed_operations.contains_key(&key) {
            return;
        }
        state.completed_operations.insert(key, response);
        state.operation_order.push_back(key);
        while state.operation_order.len() > MAX_RETAINED_KEYS {
            if let Some(expired) = state.operation_order.pop_front() {
                state.completed_operations.remove(&expired);
            }
        }
    }
}
