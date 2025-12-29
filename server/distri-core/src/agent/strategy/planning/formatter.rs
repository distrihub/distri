use std::{collections::HashSet, env, fs, sync::Arc};

use chrono::Utc;
use distri_parsers;
use distri_types::{
    ExecutionResult, MessageRole, Part, ScratchpadEntry, ScratchpadEntryType, ToolCall,
    ToolCallFormat,
};
use tracing::warn;

use crate::{
    agent::{prompt_registry::TemplateData, types::MAX_ITERATIONS, ExecutorContext},
    AgentError,
};

const SCRATCHPAD_ENTRY_LIMIT: usize = 10;

/// Helper that builds model-ready message sequences for planning prompts.
pub struct MessageFormatter<'a> {
    agent_def: &'a crate::types::StandardDefinition,
    strategy: &'a crate::types::AgentStrategy,
}

impl<'a> MessageFormatter<'a> {
    pub fn new(
        agent_def: &'a crate::types::StandardDefinition,
        strategy: &'a crate::types::AgentStrategy,
    ) -> Self {
        Self {
            agent_def,
            strategy,
        }
    }

    pub async fn build_messages(
        &self,
        message: &crate::types::Message,
        context: &Arc<ExecutorContext>,
        template: &str,
        user_template: &str,
        todos: Option<String>,
    ) -> Result<Vec<crate::types::Message>, AgentError> {
        let tool_defs = context
            .get_tools()
            .await
            .iter()
            .map(|t| t.get_tool_definition())
            .collect::<Vec<_>>();
        let available_tools = distri_parsers::get_available_tools(&tool_defs);

        let include_scratchpad = self.agent_def.include_scratchpad();
        let scratchpad_entries = if include_scratchpad {
            self.load_scratchpad_entries(context)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let native_json_tools = Self::native_json_tools(self.agent_def);

        let scratchpad = if include_scratchpad {
            self.build_scratchpad(context).await.unwrap_or_default()
        } else {
            String::new()
        };

        let hook_state = context.hook_prompt_state().await;
        let dynamic_sections = hook_state.dynamic_sections.clone();

        let reasoning_depth_name = Self::reasoning_depth_name(self.strategy);
        let execution_mode_name = Self::execution_mode_name(self.strategy);
        let tool_format_name = Self::tool_format_name(self.agent_def);

        let current_steps = context.get_usage().await.current_iteration;
        let max_steps = self.agent_def.max_iterations.unwrap_or(MAX_ITERATIONS);

        let remaining_steps = max_steps.saturating_sub(current_steps);
        let dynamic_values = hook_state.dynamic_values.clone();

        // Fetch session values from the session store
        let session_values = Self::load_session_values(context).await;

        let template_data = TemplateData {
            description: self.agent_def.description.clone(),
            instructions: self.agent_def.instructions.clone(),
            available_tools,
            task: String::new(),
            scratchpad: scratchpad.clone(),
            dynamic_sections,
            dynamic_values,
            session_values,
            reasoning_depth: reasoning_depth_name,
            execution_mode: execution_mode_name,
            tool_format: tool_format_name,
            show_examples: true,
            max_steps,
            current_steps,
            remaining_steps,
            todos: todos.clone(),
            json_tools: native_json_tools,
        };

        let template_to_use = hook_state
            .template_override
            .system
            .as_deref()
            .unwrap_or(template);
        let user_template_to_use = hook_state
            .template_override
            .user
            .as_deref()
            .unwrap_or(user_template);

        let rendered_prompt = render_prompt(context, template_to_use, &template_data).await?;

        let user_additional_data =
            render_prompt(context, user_template_to_use, &template_data).await?;
        self.log_prompt_if_needed(&rendered_prompt);

        println!("{user_additional_data}");

        let mut formatted = vec![crate::types::Message::system(rendered_prompt, None)];

        let user_message = if let Some(overrides) = &self.agent_def.user_message_overrides {
            self.build_overridden_user_message(
                message,
                overrides,
                &template_data,
                context,
                &user_additional_data,
            )
            .await?
        } else {
            Self::build_user_message(message, &user_additional_data)
        };
        formatted.push(user_message);

        if native_json_tools && include_scratchpad {
            let native_messages = Self::build_native_history_messages(&scratchpad_entries);
            formatted.extend(native_messages);
        }

        Ok(formatted)
    }

    fn reasoning_depth_name(strategy: &crate::types::AgentStrategy) -> &'static str {
        match strategy.get_reasoning_depth() {
            crate::types::ReasoningDepth::Deep => "deep",
            crate::types::ReasoningDepth::Standard => "standard",
            crate::types::ReasoningDepth::Shallow => "shallow",
        }
    }

    fn execution_mode_name(strategy: &crate::types::AgentStrategy) -> &'static str {
        match strategy.get_execution_mode() {
            crate::types::ExecutionMode::Tools => "tools",
            crate::types::ExecutionMode::Code { .. } => "code",
        }
    }

    fn tool_format_name(agent_def: &crate::types::StandardDefinition) -> &'static str {
        match agent_def.tool_format {
            ToolCallFormat::JsonL => "json",
            ToolCallFormat::Xml => "xml",
            ToolCallFormat::Code => "code",
            ToolCallFormat::Provider => "tool_calling",
            ToolCallFormat::None => "none",
        }
    }

    fn native_json_tools(agent_def: &crate::types::StandardDefinition) -> bool {
        matches!(agent_def.tool_format, ToolCallFormat::Provider)
    }

    async fn load_scratchpad_entries(
        &self,
        context: &Arc<ExecutorContext>,
    ) -> Result<Vec<ScratchpadEntry>, AgentError> {
        let Some(orchestrator) = &context.orchestrator else {
            return Ok(vec![]);
        };

        let scratchpad_store = &orchestrator.stores.scratchpad_store;

        // Subtasks: restrict to current task only to avoid polluting context with siblings/parent.
        if context.parent_task_id.is_some() {
            let entries = scratchpad_store
                .get_entries(
                    &context.thread_id,
                    &context.task_id,
                    Some(SCRATCHPAD_ENTRY_LIMIT),
                )
                .await
                .unwrap_or_default();
            return Ok(entries);
        }

        // Top-level tasks: include recent history in the thread but prefer top-level tasks first.
        // We don't have parent metadata in entries, so we just fetch thread-limited history.
        let entries = scratchpad_store
            .get_all_entries(&context.thread_id, Some(SCRATCHPAD_ENTRY_LIMIT))
            .await
            .unwrap_or_default();
        Ok(entries)
    }

    async fn build_scratchpad(&self, context: &Arc<ExecutorContext>) -> Result<String, AgentError> {
        context
            .format_agent_scratchpad(Some(SCRATCHPAD_ENTRY_LIMIT))
            .await
            .or_else(|err| {
                warn!("Falling back to inline scratchpad summary: {}", err);
                Ok(String::new())
            })
    }

    /// Load session values from the session store for this thread.
    /// These values are set by external tools and can be used in templates.
    /// The namespace is the thread_id, which matches how the frontend stores values.
    async fn load_session_values(
        context: &Arc<ExecutorContext>,
    ) -> std::collections::HashMap<String, serde_json::Value> {
        let session_store = context.get_session_store();
        match session_store {
            Ok(store) => store
                .get_all_values(&context.thread_id)
                .await
                .unwrap_or_default(),
            Err(_) => std::collections::HashMap::new(),
        }
    }

    fn build_native_history_messages(
        scratchpad_entries: &[ScratchpadEntry],
    ) -> Vec<crate::types::Message> {
        scratchpad_entries
            .iter()
            .flat_map(|entry| match &entry.entry_type {
                ScratchpadEntryType::PlanStep(_) => Vec::new(),
                ScratchpadEntryType::Execution(exec_entry) => {
                    Self::execution_result_to_messages(&exec_entry.execution_result)
                }
                ScratchpadEntryType::Task(_) => Vec::new(),
            })
            .collect()
    }

    fn execution_result_to_messages(result: &ExecutionResult) -> Vec<crate::types::Message> {
        let mut messages = Vec::new();
        let mut assistant_parts: Vec<Part> = Vec::new();
        let mut responded_tool_ids = HashSet::new();

        for part in result.parts.iter() {
            match part {
                Part::ToolResult(tool_response) => {
                    responded_tool_ids.insert(tool_response.tool_call_id.clone());
                    let mut message = crate::types::Message::tool_response(
                        tool_response.tool_call_id.clone(),
                        tool_response.tool_name.clone(),
                        &tool_response.result(),
                    );
                    message.role = MessageRole::Tool;
                    message.name = Some(tool_response.tool_name.clone());
                    message.created_at = result.timestamp;
                    message.parts = vec![Part::ToolResult(tool_response.clone())];
                    messages.push(message);
                }
                Part::ToolCall(tool_call) => {
                    assistant_parts.push(Part::ToolCall(tool_call.clone()));
                }
                Part::Text(text) => {
                    if !Self::is_observation_text(text) {
                        assistant_parts.push(Part::Text(text.clone()));
                    }
                }
                Part::Data(data) => {
                    assistant_parts.push(Part::Data(data.clone()));
                }
                Part::Image(image) => {
                    assistant_parts.push(Part::Image(image.clone()));
                }
                Part::Artifact(artifact) => {
                    assistant_parts.push(Part::Artifact(artifact.clone()));
                }
            }
        }

        if !assistant_parts.is_empty() {
            let assistant_parts = assistant_parts
                .into_iter()
                .map(|part| match part {
                    Part::ToolCall(tool_call) => {
                        if responded_tool_ids.contains(&tool_call.tool_call_id) {
                            Part::ToolCall(tool_call)
                        } else {
                            Part::Text(Self::format_tool_call(&tool_call))
                        }
                    }
                    other => other,
                })
                .collect();
            let mut assistant_message = crate::types::Message::default();
            assistant_message.role = MessageRole::Assistant;
            assistant_message.created_at = result.timestamp;
            assistant_message.parts = assistant_parts;
            messages.insert(0, assistant_message);
        }

        messages
    }

    async fn build_overridden_user_message(
        &self,
        base_message: &crate::types::Message,
        overrides: &crate::types::UserMessageOverrides,
        template_data: &TemplateData<'_>,
        context: &Arc<ExecutorContext>,
        user_additional_data: &str,
    ) -> Result<crate::types::Message, AgentError> {
        // Start with the base message parts
        let mut parts = Vec::new();

        // Add the original message parts if they exist
        if !base_message.parts.is_empty() {
            parts.extend_from_slice(&base_message.parts);
        } else if let Some(text) = base_message.as_text() {
            parts.push(Part::Text(text));
        }

        // Add parts from UserMessageOverrides
        for part_def in &overrides.parts {
            match part_def {
                distri_types::PartDefinition::Template(template_name) => {
                    let rendered = render_prompt(context, template_name, template_data).await?;
                    if !rendered.is_empty() {
                        parts.push(Part::Text(rendered));
                    }
                }
                distri_types::PartDefinition::SessionKey(key) => {
                    if let Some(value) = template_data.session_values.get(key) {
                        let resolved = Self::resolve_session_value_to_parts(
                            value,
                            overrides.include_artifacts,
                            context,
                        )
                        .await;
                        parts.extend(resolved);
                    }
                }
            }
        }

        // Add the user additional data at the end (step limit, todos, etc.)
        // Only include if include_step_count is true (default) or None (defaults to true)
        let should_include = overrides.include_step_count.unwrap_or(true);
        if should_include && !user_additional_data.is_empty() {
            parts.push(Part::Text(user_additional_data.to_string()));
        }

        let mut message = base_message.clone();
        message.role = MessageRole::User;
        message.parts = parts;
        Ok(message)
    }

    async fn resolve_session_value_to_parts(
        value: &serde_json::Value,
        force_include_artifacts: bool,
        context: &Arc<ExecutorContext>,
    ) -> Vec<Part> {
        let mut all_parts = Vec::new();
        // Try to parse as Vec<Part>, Part, or string
        if let Ok(parts) = serde_json::from_value::<Vec<Part>>(value.clone()) {
            // Vec<Part> format - expand artifacts if requested
            for part in parts {
                if force_include_artifacts {
                    // Expand artifacts to their actual content (e.g., image artifacts -> Part::Image)
                    all_parts.push(Self::load_artifact_if_needed(part, context).await);
                } else {
                    // Keep artifacts as Part::Artifact references
                    all_parts.push(part);
                }
            }
        } else if let Ok(part) = serde_json::from_value::<Part>(value.clone()) {
            // Single Part - expand artifacts if requested
            if force_include_artifacts {
                all_parts.push(Self::load_artifact_if_needed(part, context).await);
            } else {
                all_parts.push(part);
            }
        } else if let Some(text) = value.as_str() {
            all_parts.push(Part::Text(text.to_string()));
        } else {
            // For other JSON values, serialize as text
            all_parts.push(Part::Text(value.to_string()));
        }
        all_parts
    }

    /// Load artifact content if needed using ArtifactWrapper::load_artifact
    async fn load_artifact_if_needed(part: Part, context: &Arc<ExecutorContext>) -> Part {
        match part {
            Part::Artifact(metadata) => {
                // Get filesystem from orchestrator
                if let Ok(orchestrator) = context.get_orchestrator() {
                    let filesystem = orchestrator.session_filesystem.clone();
                    use distri_filesystem::ArtifactWrapper;
                    use distri_types::filesystem::FileSystemOps;
                    ArtifactWrapper::load_artifact(
                        filesystem as Arc<dyn FileSystemOps>,
                        &metadata,
                        true, // include_artifacts = true
                    )
                    .await
                } else {
                    // If no orchestrator, keep as artifact
                    Part::Artifact(metadata)
                }
            }
            other => other,
        }
    }

    fn build_user_message(
        message: &crate::types::Message,
        additional_user_data: &str,
    ) -> crate::types::Message {
        let mut user_message = message.clone();
        user_message.role = MessageRole::User;
        if user_message.parts.is_empty() {
            if let Some(text) = message.as_text() {
                user_message.parts.push(Part::Text(text));
            }
        }

        if !additional_user_data.is_empty() {
            user_message
                .parts
                .push(Part::Text(additional_user_data.to_string()));
        }
        user_message
    }

    fn format_tool_call(tool_call: &ToolCall) -> String {
        format!(
            "Tool Call -> {} with input: {}",
            tool_call.tool_name,
            serde_json::to_string(&tool_call.input).unwrap_or_else(|_| "{}".to_string())
        )
    }

    fn is_observation_text(text: &str) -> bool {
        let trimmed = text.trim();
        trimmed.starts_with("Observation:") || trimmed.starts_with("Action:")
    }

    fn log_prompt_if_needed(&self, prompt: &str) {
        if let Ok(agent_filter) = env::var("DISTRI_LOG_PROMPT") {
            if agent_filter == "*" || agent_filter == self.agent_def.name {
                log_prompt(&self.agent_def.name, prompt);
            }
        }
    }
}

fn log_prompt(agent_id: &str, prompt: &str) {
    let timestamp = Utc::now().timestamp_millis().to_string();
    if let Err(e) = fs::create_dir_all(".distri/prompts") {
        warn!("Failed to create prompt directory: {}", e);
        return;
    }
    if let Err(e) = fs::write(
        format!(".distri/prompts/{agent_id}_{timestamp}.json"),
        prompt,
    ) {
        warn!("Failed to write prompt log: {}", e);
    }
}

async fn render_prompt(
    context: &Arc<ExecutorContext>,
    template: &str,
    template_data: &TemplateData<'_>,
) -> Result<String, AgentError> {
    if let Some(orchestrator) = &context.orchestrator {
        let prompt_registry = orchestrator.get_prompt_registry();
        let rendered_prompt = prompt_registry
            .render_template(template, template_data)
            .await?;
        Ok(rendered_prompt)
    } else {
        Ok(template.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentStrategy, Message};
    use distri_types::{
        Action, ExecutionHistoryEntry, ExecutionStatus, ModelProvider, ModelSettings, PlanStep,
        ToolCall, ToolResponse,
    };
    use serde_json::json;

    fn base_agent_definition(
        provider: ModelProvider,
        format: ToolCallFormat,
    ) -> crate::types::StandardDefinition {
        crate::types::StandardDefinition {
            name: "test".to_string(),
            instructions: "Be helpful".to_string(),
            model_settings: ModelSettings {
                provider,
                ..ModelSettings::default()
            },
            tool_format: format,
            ..Default::default()
        }
    }

    fn sample_execution_result() -> ExecutionResult {
        let tool_call = ToolCall {
            tool_call_id: "call-1".to_string(),
            tool_name: "apply_ops".to_string(),
            input: json!({"ops": []}),
        };
        let tool_response = ToolResponse::direct(
            "call-1".to_string(),
            "apply_ops".to_string(),
            json!({"result": {"success": true}}),
        );

        ExecutionResult {
            step_id: "step-1".to_string(),
            parts: vec![Part::ToolCall(tool_call), Part::ToolResult(tool_response)],
            status: ExecutionStatus::Success,
            reason: None,
            timestamp: 1,
        }
    }

    fn sample_plan_entry() -> ScratchpadEntry {
        ScratchpadEntry {
            timestamp: 0,
            entry_type: ScratchpadEntryType::PlanStep(PlanStep {
                id: "step-1".to_string(),
                thought: Some("Think".to_string()),
                action: Action::ToolCalls {
                    tool_calls: vec![ToolCall {
                        tool_call_id: "call-1".to_string(),
                        tool_name: "search".to_string(),
                        input: json!({"query": "rust"}),
                    }],
                },
            }),
            task_id: "task".to_string(),
            parent_task_id: None,
            entry_kind: Some("task".to_string()),
        }
    }

    fn sample_execution_entry() -> ScratchpadEntry {
        ScratchpadEntry {
            timestamp: 1,
            entry_type: ScratchpadEntryType::Execution(ExecutionHistoryEntry {
                thread_id: "thread".to_string(),
                task_id: "task".to_string(),
                run_id: "run".to_string(),
                execution_result: sample_execution_result(),
                stored_at: 1,
            }),
            task_id: "task".to_string(),
            parent_task_id: None,
            entry_kind: Some("task".to_string()),
        }
    }

    #[tokio::test]
    async fn native_history_uses_scratchpad_entries() {
        let native = MessageFormatter::build_native_history_messages(&[
            sample_plan_entry(),
            sample_execution_entry(),
        ]);

        assert_eq!(native.len(), 2);
        assert!(matches!(native[0].role, MessageRole::Assistant));
        assert!(matches!(native[1].role, MessageRole::Tool));
        assert_eq!(native[1].tool_responses().len(), 1);
        assert_eq!(native[0].tool_calls().len(), 1);
    }

    #[tokio::test]
    async fn fallback_history_from_execution_results() {
        let messages = MessageFormatter::build_native_history_messages(&[]);
        assert_eq!(messages.len(), 2);
        assert!(matches!(messages[0].role, MessageRole::Assistant));
        assert!(matches!(messages[1].role, MessageRole::Tool));
    }

    #[tokio::test]
    async fn openai_messages_include_tool_history_when_native() {
        let agent_def = base_agent_definition(ModelProvider::OpenAI {}, ToolCallFormat::Provider);
        let strategy = AgentStrategy::default();
        let formatter = MessageFormatter::new(&agent_def, &strategy);
        let context = Arc::new(ExecutorContext::default());
        let user_msg = Message::user("Plan".to_string(), None);

        context
            .store_execution_result(&sample_execution_result())
            .await
            .unwrap();
        let messages = formatter
            .build_messages(&user_msg, &context, "tmpl", "user_templ", None)
            .await
            .expect("formatter should succeed");

        assert_eq!(messages.len(), 4);
        assert!(matches!(messages[0].role, MessageRole::System));
        assert!(matches!(messages[1].role, MessageRole::User));
        let user_text = messages[1].as_text().unwrap_or_default();
        assert!(user_text.contains("Steps remaining"));
        assert!(matches!(messages[2].role, MessageRole::Assistant));
        assert!(matches!(messages[3].role, MessageRole::Tool));
        assert_eq!(messages[2].tool_calls().len(), 1);
        assert_eq!(messages[3].tool_responses().len(), 1);
    }

    #[tokio::test]
    async fn non_openai_prefers_system_and_user_only() {
        let agent_def = base_agent_definition(ModelProvider::OpenAI {}, ToolCallFormat::JsonL);
        let strategy = AgentStrategy::default();
        let formatter = MessageFormatter::new(&agent_def, &strategy);
        let context = Arc::new(ExecutorContext::default());
        let user_msg = Message::user("Summarize context".to_string(), None);
        context
            .store_execution_result(&sample_execution_result())
            .await
            .unwrap();
        let messages = formatter
            .build_messages(
                &user_msg,
                &context,
                "tmpl",
                "user_templ",
                Some("Todo".to_string()),
            )
            .await
            .expect("formatter should succeed");

        assert_eq!(messages.len(), 2);
        assert!(matches!(messages[0].role, MessageRole::System));
        assert!(matches!(messages[1].role, MessageRole::User));
        let user_text = messages[1].as_text().unwrap_or_default();
        assert!(user_text.contains("Steps remaining"));
        assert!(user_text.contains("Todos"));
    }
}
