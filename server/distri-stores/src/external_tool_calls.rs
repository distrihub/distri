use async_trait::async_trait;
use distri_types::{ToolResponse, stores::ExternalToolCallsStore};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{RwLock, oneshot};

/// In-memory implementation using HashMap and oneshot channels
#[derive(Clone)]
pub struct InMemoryExternalToolCallsStore {
    tool_calls: Arc<RwLock<HashMap<String, oneshot::Sender<ToolResponse>>>>,
}

impl std::fmt::Debug for InMemoryExternalToolCallsStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryExternalToolCallsStore")
            .field("sessions", &"<oneshot channels>")
            .finish()
    }
}

impl Default for InMemoryExternalToolCallsStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryExternalToolCallsStore {
    pub fn new() -> Self {
        Self {
            tool_calls: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl ExternalToolCallsStore for InMemoryExternalToolCallsStore {
    async fn register_external_tool_call(
        &self,
        tool_call_id: &str,
    ) -> anyhow::Result<oneshot::Receiver<ToolResponse>> {
        let (tx, rx) = oneshot::channel();

        let mut sessions = self.tool_calls.write().await;
        if sessions.contains_key(tool_call_id) {
            return Err(anyhow::anyhow!("Session {} already exists", tool_call_id));
        }

        sessions.insert(tool_call_id.to_string(), tx);
        tracing::debug!("Registered external tool call session: {}", tool_call_id);

        Ok(rx)
    }

    async fn complete_external_tool_call(
        &self,
        tool_call_id: &str,
        tool_response: ToolResponse,
    ) -> anyhow::Result<()> {
        let sender = {
            let mut sessions = self.tool_calls.write().await;
            sessions.remove(tool_call_id)
        };

        match sender {
            Some(sender) => {
                if let Err(_) = sender.send(tool_response) {
                    return Err(anyhow::anyhow!(
                        "Failed to send tool response for session {} - receiver may have been dropped",
                        tool_call_id
                    ));
                }
                tracing::debug!("Completed external tool call for session: {}", tool_call_id);
                Ok(())
            }
            None => Err(anyhow::anyhow!(
                "No pending external tool execution found for session: {}",
                tool_call_id
            )),
        }
    }

    async fn remove_tool_call(&self, tool_call_id: &str) -> anyhow::Result<()> {
        let mut sessions = self.tool_calls.write().await;
        sessions.remove(tool_call_id);
        tracing::debug!("Removed external tool call session: {}", tool_call_id);
        Ok(())
    }

    async fn list_pending_tool_calls(&self) -> anyhow::Result<Vec<String>> {
        let sessions = self.tool_calls.read().await;
        Ok(sessions.keys().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::Part;

    #[tokio::test]
    async fn test_in_memory_store_basic_flow() {
        let store = InMemoryExternalToolCallsStore::new();
        let tool_call_id = "test_session_123";

        // Register a session
        let rx = store
            .register_external_tool_call(tool_call_id)
            .await
            .unwrap();

        // Check that it's pending
        let pending = store.list_pending_tool_calls().await.unwrap();
        assert!(pending.contains(&tool_call_id.to_string()));

        // Complete the tool call
        let response = ToolResponse {
            tool_call_id: "tool_123".to_string(),
            tool_name: "test_tool".to_string(),
            parts: vec![Part::Text("test response".to_string())],
        };

        store
            .complete_external_tool_call(tool_call_id, response.clone())
            .await
            .unwrap();

        // Should receive the response
        let received = rx.await.unwrap();
        assert_eq!(received.tool_call_id, response.tool_call_id);
        assert_eq!(received.tool_name, response.tool_name);

        // Session should be cleaned up
        let pending_after = store.list_pending_tool_calls().await.unwrap();
        assert!(!pending_after.contains(&tool_call_id.to_string()));
    }

    #[tokio::test]
    async fn test_in_memory_store_duplicate_session() {
        let store = InMemoryExternalToolCallsStore::new();
        let tool_call_id = "duplicate_session";

        // Register first session
        let _rx1 = store
            .register_external_tool_call(tool_call_id)
            .await
            .unwrap();

        // Try to register same session again - should fail
        let result = store.register_external_tool_call(tool_call_id).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[tokio::test]
    async fn test_in_memory_store_complete_nonexistent() {
        let store = InMemoryExternalToolCallsStore::new();
        let tool_call_id = "nonexistent_session";

        let response = ToolResponse {
            tool_call_id: "tool_123".to_string(),
            tool_name: "test_tool".to_string(),
            parts: vec![Part::Text("test response".to_string())],
        };

        // Try to complete non-existent session - should fail
        let result = store
            .complete_external_tool_call(tool_call_id, response)
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No pending external tool execution")
        );
    }
}
