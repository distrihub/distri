//! Helper constructors that build typed span attribute structs from distri runtime types.
//!
//! `ContextFields` is a borrowed view of ExecutorContext fields, allowing llm-gateway
//! to build span structs without depending on distri-core's ExecutorContext directly.

use crate::observability::types::{
    GenAiAgentSpan, GenAiInferenceSpan, GenAiOperation, GenAiToolSpan, GenAiToolType,
};
use distri_types::ModelSettings;

/// Lightweight borrowed view of ExecutorContext fields needed for span creation.
/// Callers (in distri-core) extract these fields and pass them here, avoiding
/// a direct llm-gateway → distri-core dependency.
pub struct ContextFields<'a> {
    pub thread_id: &'a str,
    pub task_id: &'a str,
    pub run_id: &'a str,
    pub agent_id: &'a str,
    pub user_id: &'a str,
    pub workspace_id: Option<&'a str>,
    pub channel_id: Option<&'a str>,
}

impl GenAiInferenceSpan {
    /// Build from ModelSettings + context fields.
    pub fn from_model_settings(ms: &ModelSettings, ctx: &ContextFields<'_>) -> Self {
        Self {
            operation: Some(GenAiOperation::Chat),
            provider: Some(ms.inner.provider.otel_provider_name().to_string()),
            request_model: Some(ms.model.clone()),
            conversation_id: Some(ctx.thread_id.to_string()),
            temperature: ms.inner.temperature.map(|t| t as f64),
            max_tokens: ms.inner.max_tokens.map(|m| m as i64),
            distri_thread_id: Some(ctx.thread_id.to_string()),
            distri_workspace_id: ctx.workspace_id.map(str::to_string),
            distri_task_id: Some(ctx.task_id.to_string()),
            distri_run_id: Some(ctx.run_id.to_string()),
            distri_agent_id: Some(ctx.agent_id.to_string()),
            distri_user_id: Some(ctx.user_id.to_string()),
            distri_channel_id: ctx.channel_id.map(str::to_string),
            ..Default::default()
        }
    }
}

impl GenAiAgentSpan {
    /// Build from context fields and an optional parent agent ID (for sub-agent spans).
    pub fn from_context_fields(
        agent_name: &str,
        ctx: &ContextFields<'_>,
        parent_agent_id: Option<&str>,
    ) -> Self {
        Self {
            agent_id: Some(ctx.agent_id.to_string()),
            agent_name: agent_name.to_string(),
            parent_agent_id: parent_agent_id.map(str::to_string),
            conversation_id: Some(ctx.thread_id.to_string()),
            distri_thread_id: Some(ctx.thread_id.to_string()),
            distri_workspace_id: ctx.workspace_id.map(str::to_string),
            distri_task_id: Some(ctx.task_id.to_string()),
            distri_run_id: Some(ctx.run_id.to_string()),
            distri_user_id: Some(ctx.user_id.to_string()),
            distri_channel_id: ctx.channel_id.map(str::to_string),
            ..Default::default()
        }
    }
}

impl GenAiToolSpan {
    /// Build from tool execution event fields.
    pub fn from_event_fields(
        tool_name: &str,
        tool_call_id: &str,
        step_id: &str,
        ctx: &ContextFields<'_>,
    ) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            tool_type: Some(GenAiToolType::Function),
            tool_call_id: Some(tool_call_id.to_string()),
            distri_thread_id: Some(ctx.thread_id.to_string()),
            distri_task_id: Some(ctx.task_id.to_string()),
            distri_step_id: Some(step_id.to_string()),
            distri_agent_id: Some(ctx.agent_id.to_string()),
            distri_run_id: Some(ctx.run_id.to_string()),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx<'a>() -> ContextFields<'a> {
        ContextFields {
            thread_id: "thread-1",
            task_id: "task-1",
            run_id: "run-1",
            agent_id: "coder",
            user_id: "user-1",
            workspace_id: Some("ws-1"),
            channel_id: None,
        }
    }

    #[test]
    fn agent_span_from_context_fields() {
        // ModelSettings is from distri-types; create a minimal one
        // We need to check what ModelSettings looks like — it may not have a simple constructor.
        // If ModelSettings construction is complex, just test ContextFields and the other two impls.
        let ctx = test_ctx();
        let agent = GenAiAgentSpan::from_context_fields("coder", &ctx, None);
        assert_eq!(agent.agent_name, "coder");
        assert_eq!(agent.distri_thread_id, Some("thread-1".to_string()));
        assert_eq!(agent.distri_run_id, Some("run-1".to_string()));
        assert!(agent.parent_agent_id.is_none());
    }

    #[test]
    fn agent_span_with_parent() {
        let ctx = test_ctx();
        let agent = GenAiAgentSpan::from_context_fields("sub-agent", &ctx, Some("parent-id"));
        assert_eq!(agent.parent_agent_id, Some("parent-id".to_string()));
        assert_eq!(agent.conversation_id, Some("thread-1".to_string()));
    }

    #[test]
    fn tool_span_from_event_fields() {
        let ctx = test_ctx();
        let tool = GenAiToolSpan::from_event_fields("bash", "tc-123", "step-1", &ctx);
        assert_eq!(tool.tool_name, "bash");
        assert_eq!(tool.tool_call_id, Some("tc-123".to_string()));
        assert_eq!(tool.distri_step_id, Some("step-1".to_string()));
        assert!(matches!(tool.tool_type, Some(GenAiToolType::Function)));
    }

    #[test]
    fn workspace_and_channel_optional() {
        let ctx = ContextFields {
            thread_id: "t",
            task_id: "task",
            run_id: "run",
            agent_id: "ag",
            user_id: "u",
            workspace_id: None,
            channel_id: None,
        };
        let agent = GenAiAgentSpan::from_context_fields("ag", &ctx, None);
        assert!(agent.distri_workspace_id.is_none());
        assert!(agent.distri_channel_id.is_none());
    }
}
