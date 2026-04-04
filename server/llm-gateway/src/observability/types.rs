//! Typed structs for OpenTelemetry GenAI semantic conventions (2025, dev stability).
//! https://opentelemetry.io/docs/specs/semconv/gen-ai/

/// gen_ai.operation.name values
#[derive(Debug, Clone, PartialEq)]
pub enum GenAiOperation {
    Chat,
    InvokeAgent,
    ExecuteTool,
    Embeddings,
}

impl GenAiOperation {
    pub fn as_str(&self) -> &str {
        match self {
            GenAiOperation::Chat => "chat",
            GenAiOperation::InvokeAgent => "invoke_agent",
            GenAiOperation::ExecuteTool => "execute_tool",
            GenAiOperation::Embeddings => "embeddings",
        }
    }
}

/// gen_ai.tool.type values
#[derive(Debug, Clone, PartialEq)]
pub enum GenAiToolType {
    Function,
    Extension,
    Datastore,
}

impl GenAiToolType {
    pub fn as_str(&self) -> &str {
        match self {
            GenAiToolType::Function => "function",
            GenAiToolType::Extension => "extension",
            GenAiToolType::Datastore => "datastore",
        }
    }
}

/// All attributes for an LLM inference span.
/// Span name: "{operation} {request_model}"  e.g. "chat claude-3-5-sonnet-20241022"
#[derive(Debug, Default, Clone)]
pub struct GenAiInferenceSpan {
    pub operation: Option<GenAiOperation>,
    /// OTel `gen_ai.provider.name` value (e.g. "anthropic", "openai", "azure.ai.openai").
    /// Use `ModelProvider::otel_provider_name()` from distri-types to populate this.
    pub provider: Option<String>,
    pub request_model: Option<String>,
    pub response_model: Option<String>,
    pub conversation_id: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<i64>,
    pub top_p: Option<f64>,
    pub stream: bool,
    pub response_id: Option<String>,
    pub finish_reasons: Vec<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub cache_creation_input_tokens: Option<i64>,
    pub estimated_cost_usd: Option<f64>,
    pub distri_thread_id: Option<String>,
    pub distri_workspace_id: Option<String>,
    pub distri_task_id: Option<String>,
    pub distri_run_id: Option<String>,
    pub distri_agent_id: Option<String>,
    pub distri_user_id: Option<String>,
    pub distri_channel_id: Option<String>,
}

impl GenAiInferenceSpan {
    pub fn span_name(&self) -> String {
        let op = self
            .operation
            .as_ref()
            .map(|o| o.as_str())
            .unwrap_or("chat");
        let model = self.request_model.as_deref().unwrap_or("unknown");
        format!("{} {}", op, model)
    }
}

/// All attributes for an agent lifecycle span.
/// Span name: "invoke_agent {agent_name}"
#[derive(Debug, Default, Clone)]
pub struct GenAiAgentSpan {
    pub agent_id: Option<String>,
    pub agent_name: String,
    pub parent_agent_id: Option<String>,
    pub conversation_id: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub estimated_cost_usd: Option<f64>,
    pub distri_thread_id: Option<String>,
    pub distri_workspace_id: Option<String>,
    pub distri_task_id: Option<String>,
    pub distri_run_id: Option<String>,
    pub distri_user_id: Option<String>,
    pub distri_channel_id: Option<String>,
}

impl GenAiAgentSpan {
    pub fn span_name(&self) -> String {
        format!("invoke_agent {}", self.agent_name)
    }
}

/// All attributes for a tool execution span.
/// Span name: "execute_tool {tool_name}"
#[derive(Debug, Default, Clone)]
pub struct GenAiToolSpan {
    pub tool_name: String,
    pub tool_type: Option<GenAiToolType>,
    pub tool_call_id: Option<String>,
    pub tool_description: Option<String>,
    pub success: Option<bool>,
    pub distri_thread_id: Option<String>,
    pub distri_task_id: Option<String>,
    pub distri_step_id: Option<String>,
    pub distri_agent_id: Option<String>,
    pub distri_run_id: Option<String>,
}

impl GenAiToolSpan {
    pub fn span_name(&self) -> String {
        format!("execute_tool {}", self.tool_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_display() {
        assert_eq!(GenAiOperation::Chat.as_str(), "chat");
        assert_eq!(GenAiOperation::InvokeAgent.as_str(), "invoke_agent");
        assert_eq!(GenAiOperation::ExecuteTool.as_str(), "execute_tool");
    }

    #[test]
    fn tool_type_display() {
        assert_eq!(GenAiToolType::Function.as_str(), "function");
    }

    #[test]
    fn span_names() {
        let inf = GenAiInferenceSpan {
            operation: Some(GenAiOperation::Chat),
            request_model: Some("claude-3-5-sonnet-20241022".into()),
            provider: Some("anthropic".to_string()),
            ..Default::default()
        };
        assert_eq!(inf.span_name(), "chat claude-3-5-sonnet-20241022");

        let agent = GenAiAgentSpan {
            agent_name: "coder".into(),
            ..Default::default()
        };
        assert_eq!(agent.span_name(), "invoke_agent coder");

        let tool = GenAiToolSpan {
            tool_name: "bash".into(),
            ..Default::default()
        };
        assert_eq!(tool.span_name(), "execute_tool bash");
    }

}
