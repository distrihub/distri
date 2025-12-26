use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use anyhow::Result;
use distri_types::{AgentEvent, ToolCall, ToolResponse};

type HandlerFuture = Pin<Box<dyn Future<Output = Result<ToolResponse>> + Send>>;
type Handler = Arc<dyn Fn(ToolCall, AgentEvent) -> HandlerFuture + Send + Sync + 'static>;

/// Per-context registry for handling external tools locally instead of waiting for remote completion.
#[derive(Clone, Default)]
pub struct ExternalToolRegistry {
    handlers: Arc<std::sync::RwLock<HashMap<(String, String), Handler>>>,
}

impl ExternalToolRegistry {
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(std::sync::RwLock::new(HashMap::new())),
        }
    }

    /// Register a handler for a specific agent and tool name.
    pub fn register<F, Fut>(
        &self,
        agent: impl Into<String>,
        tool_name: impl Into<String>,
        handler: F,
    ) where
        F: Fn(ToolCall, AgentEvent) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ToolResponse>> + Send + 'static,
    {
        let wrapped = Arc::new(move |call: ToolCall, event: AgentEvent| -> HandlerFuture {
            Box::pin(handler(call, event))
        });
        if let Ok(mut guard) = self.handlers.write() {
            guard.insert((agent.into(), tool_name.into()), wrapped);
        }
    }

    /// Merge handlers from another registry (used when forking/cloning contexts).
    pub fn merge_from(&self, other: &ExternalToolRegistry) {
        if let (Ok(mut ours), Ok(theirs)) = (self.handlers.write(), other.handlers.read()) {
            for (k, v) in theirs.iter() {
                ours.insert(k.clone(), v.clone());
            }
        }
    }

    /// Try to handle a tool; returns a ToolResponse if a handler exists.
    pub async fn try_handle(
        &self,
        agent: &str,
        tool_name: &str,
        call: &ToolCall,
        event: &AgentEvent,
    ) -> Option<Result<ToolResponse>> {
        let guard = self.handlers.read().ok()?;
        let key = (agent.to_string(), tool_name.to_string());
        let handler = guard
            .get(&key)
            .or_else(|| guard.get(&("*".to_string(), tool_name.to_string())))?
            .clone();
        Some(handler(call.clone(), event.clone()).await)
    }

    /// Check if a tool handler is registered for the agent or globally.
    pub fn has_tool(&self, agent: &str, tool_name: &str) -> bool {
        let Ok(guard) = self.handlers.read() else {
            return false;
        };
        guard.contains_key(&(agent.to_string(), tool_name.to_string()))
            || guard.contains_key(&("*".to_string(), tool_name.to_string()))
    }
}
