use std::{collections::HashSet, env, fs, sync::Arc};

use chrono::Utc;
use distri_parsers;
use distri_types::{
    ContextBudget, ExecutionResult, MessageRole, Part, ScratchpadEntry, ScratchpadEntryType,
    ToolCall, ToolCallFormat,
};
use tracing::warn;

use crate::{
    agent::{
        prompt_registry::TemplateData, token_estimator::TokenEstimator, types::MAX_ITERATIONS,
        ExecutorContext,
    },
    AgentError,
};

const MIN_SCRATCHPAD_ENTRY_LIMIT: usize = 10;
const MAX_SCRATCHPAD_ENTRY_LIMIT: usize = 100;

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
    ) -> Result<(Vec<crate::types::Message>, ContextBudget), AgentError> {
        let tools = context.get_tools().await;
        let tool_defs = tools
            .iter()
            .map(|t| t.get_tool_definition())
            .collect::<Vec<_>>();
        let available_tools = distri_parsers::get_available_tools(&tool_defs);

        let include_scratchpad = self.agent_def.include_scratchpad();
        let scratchpad_entry_limit = Self::scratchpad_entry_limit(self.agent_def);
        let scratchpad_entries = if include_scratchpad {
            self.load_scratchpad_entries(context, scratchpad_entry_limit)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let native_json_tools = Self::native_json_tools(self.agent_def);

        let scratchpad = if include_scratchpad {
            self.build_scratchpad(context, scratchpad_entry_limit)
                .await
                .unwrap_or_default()
        } else {
            String::new()
        };

        let hook_state = context.hook_prompt_state().await;
        let dynamic_sections = hook_state.dynamic_sections.clone();

        let reasoning_depth_name = Self::reasoning_depth_name(self.strategy);
        let execution_mode_name = Self::execution_mode_name(self.strategy);
        let tool_format_name = Self::tool_format_name(self.agent_def);
        let runtime_mode_name = Self::runtime_mode_name(&context.runtime_mode);

        let current_steps = context.get_usage().await.current_iteration;
        let max_steps = self.agent_def.max_iterations.unwrap_or(MAX_ITERATIONS);

        let remaining_steps = max_steps.saturating_sub(current_steps);
        let mut dynamic_values = hook_state.dynamic_values.clone();

        // Fetch session values from the session store
        let session_values = Self::load_session_values(context).await;

        // Extract available_skills from dynamic_values if present
        let available_skills = dynamic_values
            .remove("available_skills")
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        // Extract deferred_tools_listing injected by orchestrator
        let deferred_tools_listing = dynamic_values
            .remove("deferred_tools_listing")
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        // Get deferred tool names so we can skip their prompts
        let deferred_names = context.get_deferred_tool_names().await;

        // Collect per-tool prompts only for NON-deferred tools.
        // Deferred tools shouldn't have prompts in the system prompt — they'll
        // get their prompts when loaded via tool_search exact name match.
        let (tool_prompts_map, tool_prompts_concat, tool_prompt_entries) =
            Self::collect_tool_prompts(&tool_defs, &deferred_names);

        tracing::info!(
            "Tool prompts injected for: {:?} (total tools: {}, deferred: {})",
            tool_prompts_map.keys().collect::<Vec<_>>(),
            tool_defs.len(),
            deferred_names.len()
        );

        // Inject tool prompts as {{tools.Bash}}, {{tools.Read}}, etc.
        // Always inject (even if empty) so handlebars strict mode doesn't fail.
        dynamic_values.insert(
            "tools".to_string(),
            serde_json::Value::Object(tool_prompts_map),
        );

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
            available_skills,
            tool_prompts: tool_prompts_concat,
            tool_prompt_list: tool_prompt_entries,
            deferred_tools_listing,
            channel_kind: context.channel_kind.clone(),
            runtime_mode: runtime_mode_name,
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

        // Compute rendered prompt token count before moving the string.
        let rendered_prompt_tokens = TokenEstimator::rough_token_count(&rendered_prompt);

        let mut formatted = vec![crate::types::Message::system(rendered_prompt, None)];

        // Build the current user message with any dynamic additions (step limit, todos, etc.).
        // We'll also use this to "upsert" the current user message into the task history,
        // ensuring it appears only once and does not always get appended at the very end.
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

        let user_history = Self::load_task_user_messages(context).await;
        let tool_history = if native_json_tools && include_scratchpad {
            Self::build_native_history_messages(&scratchpad_entries)
        } else {
            Vec::new()
        };

        let interleaved_history =
            Self::interleave_user_and_tool_history(user_history, tool_history, &user_message);
        formatted.extend(interleaved_history);

        // Fallback: if we couldn't load history (no orchestrator/store), still include the user.
        if !formatted.iter().any(|m| m.id == user_message.id) {
            formatted.push(user_message);
        }

        // Compute context budget breakdown from the built prompt components.
        let budget = {
            // System prompt: static portion = agent instructions (fixed across sessions)
            let system_prompt_static_tokens =
                TokenEstimator::rough_token_count(&self.agent_def.instructions);

            // Dynamic portion = rendered prompt minus the static instructions size
            let system_prompt_dynamic_tokens =
                rendered_prompt_tokens.saturating_sub(system_prompt_static_tokens);

            // Tool schema tokens: sum of non-deferred tool JSON definitions
            let tool_schema_tokens: usize = tool_defs
                .iter()
                .filter(|def| !deferred_names.contains(&def.name))
                .map(|def| {
                    let json = serde_json::to_string(def).unwrap_or_default();
                    TokenEstimator::rough_token_count(&json)
                })
                .sum();

            // Deferred tool listing tokens
            let deferred_tool_tokens = template_data
                .deferred_tools_listing
                .as_deref()
                .map(TokenEstimator::rough_token_count)
                .unwrap_or(0);

            // Skill listing tokens
            let skill_listing_tokens = template_data
                .available_skills
                .as_deref()
                .map(TokenEstimator::rough_token_count)
                .unwrap_or(0);

            // Conversation history: all non-system messages in `formatted`
            let conversation_tokens: usize = formatted
                .iter()
                .filter(|m| !matches!(m.role, MessageRole::System))
                .map(|m| {
                    let text = m.as_text().unwrap_or_default();
                    TokenEstimator::rough_token_count(&text)
                })
                .sum();

            // Tool result tokens: from ToolResult parts in history
            let tool_result_tokens: usize = formatted
                .iter()
                .flat_map(|m| &m.parts)
                .filter_map(|p| {
                    if let Part::ToolResult(tr) = p {
                        let json = serde_json::to_string(&tr.result()).unwrap_or_default();
                        Some(TokenEstimator::rough_token_count(&json))
                    } else {
                        None
                    }
                })
                .sum();

            // Context window size from agent definition
            let context_window_size = self.agent_def.get_effective_context_size() as usize;

            ContextBudget {
                system_prompt_static_tokens,
                system_prompt_dynamic_tokens,
                tool_schema_tokens,
                deferred_tool_tokens,
                skill_listing_tokens,
                conversation_tokens,
                tool_result_tokens,
                context_window_size,
                static_prefix_cache_hit: false,
                static_prefix_hash: None,
            }
        };

        Ok((formatted, budget))
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

    /// Lower-snake-case name for `RuntimeMode`, matching the
    /// `#[serde(rename_all = "snake_case")]` form used everywhere the mode
    /// crosses a wire (agent definition `runtime` field, span attributes,
    /// telemetry). Templates branch off this string via
    /// `{{#if (eq runtime_mode "cli")}}` / `{{#if (eq runtime_mode "cloud")}}`.
    fn runtime_mode_name(mode: &distri_types::RuntimeMode) -> &'static str {
        match mode {
            distri_types::RuntimeMode::Cli => "cli",
            distri_types::RuntimeMode::Cloud => "cloud",
            distri_types::RuntimeMode::Browser => "browser",
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

    /// Collect per-tool prompt instructions, skipping deferred tools.
    ///
    /// Returns:
    /// - JSON map for `{{tools.Bash}}` etc. (all tools get an entry, deferred ones get "")
    /// - Concatenated prompt string for `{{{tool_prompts}}}`
    /// - Vec of ToolPromptEntry for `{{#each tool_prompt_list}}`
    ///
    /// Deferred tool prompts are NOT included in the system prompt — they're
    /// delivered when the model loads the tool via `tool_search` exact name match.
    fn collect_tool_prompts(
        tool_defs: &[distri_types::ToolDefinition],
        deferred_names: &std::collections::HashSet<String>,
    ) -> (
        serde_json::Map<String, serde_json::Value>,
        String,
        Vec<distri_types::prompt::ToolPromptEntry>,
    ) {
        let mut map = serde_json::Map::new();
        let mut concat = String::new();
        let mut entries = Vec::new();

        for def in tool_defs {
            let is_deferred = deferred_names.contains(&def.name);

            if is_deferred {
                // Deferred tools get an empty entry so {{tools.X}} doesn't error
                map.insert(def.name.clone(), serde_json::Value::String(String::new()));
                continue;
            }

            let prompt = def.prompt.clone().unwrap_or_default();
            map.insert(def.name.clone(), serde_json::Value::String(prompt.clone()));

            if !prompt.is_empty() {
                // Add to concatenated string
                concat.push_str(&format!("## {}\n{}\n\n", def.name, prompt));
                // Add to iterable list
                entries.push(distri_types::prompt::ToolPromptEntry {
                    name: def.name.clone(),
                    prompt,
                });
            }
        }

        (map, concat, entries)
    }

    fn scratchpad_entry_limit(agent_def: &crate::types::StandardDefinition) -> usize {
        let history_size = agent_def.history_size.unwrap_or(5);
        let suggested = history_size.saturating_mul(10);
        suggested.clamp(MIN_SCRATCHPAD_ENTRY_LIMIT, MAX_SCRATCHPAD_ENTRY_LIMIT)
    }

    /// Always scoped to the current task. Each task — top-level or
    /// subtask — owns its own scratchpad. Pulling thread-wide history
    /// into a top-level task pollutes the LLM's context with SIBLING
    /// tasks' partial reasoning, which manifested as the run_skill
    /// → run_skill recursion in the original fork bug. With the
    /// Invocation model, every task that wants conversation context
    /// reads it via task_messages (the message history), not via
    /// peer tasks' scratchpads.
    async fn load_scratchpad_entries(
        &self,
        context: &Arc<ExecutorContext>,
        limit: usize,
    ) -> Result<Vec<ScratchpadEntry>, AgentError> {
        let Some(orchestrator) = &context.orchestrator else {
            return Ok(vec![]);
        };

        let entries = orchestrator
            .stores
            .scratchpad_store
            .get_entries(&context.thread_id, &context.task_id, Some(limit))
            .await
            .unwrap_or_default();

        tracing::info!(
            target: "scratchpad.load",
            agent = %context.agent_id,
            task_id = %context.task_id,
            parent_task_id = %context.parent_task_id.as_deref().unwrap_or("<top-level>"),
            count = entries.len(),
            "scratchpad load"
        );
        Ok(entries)
    }

    async fn build_scratchpad(
        &self,
        context: &Arc<ExecutorContext>,
        limit: usize,
    ) -> Result<String, AgentError> {
        context
            .format_agent_scratchpad(Some(limit))
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

    async fn load_task_user_messages(context: &Arc<ExecutorContext>) -> Vec<crate::types::Message> {
        let Ok(history) = context.get_current_task_message_history().await else {
            return Vec::new();
        };

        let mut user_messages: Vec<_> = history
            .into_iter()
            .filter(|message| matches!(message.role, MessageRole::User))
            .collect();

        user_messages.sort_by_key(|msg| msg.created_at);
        user_messages
    }

    fn interleave_user_and_tool_history(
        user_messages: Vec<crate::types::Message>,
        tool_messages: Vec<crate::types::Message>,
        current_user_message: &crate::types::Message,
    ) -> Vec<crate::types::Message> {
        // Replace any stored version of the current user message with the enriched one
        // (includes step limits/todos/etc.), but keep its original timestamp.
        let mut users: Vec<_> = user_messages
            .into_iter()
            .filter(|m| m.id != current_user_message.id)
            .collect();
        users.push(current_user_message.clone());
        users.sort_by_key(|msg| msg.created_at);

        // Keep tool messages ordered, but stable within the same timestamp.
        let mut tools_with_index: Vec<(usize, crate::types::Message)> =
            tool_messages.into_iter().enumerate().collect();
        tools_with_index.sort_by(|(a_idx, a_msg), (b_idx, b_msg)| {
            a_msg
                .created_at
                .cmp(&b_msg.created_at)
                .then(a_idx.cmp(b_idx))
        });

        let mut interleaved = Vec::new();
        let mut tool_cursor = 0;

        // Include any tool messages that (for whatever reason) predate the first user message.
        if let Some(first_user) = users.first() {
            while tool_cursor < tools_with_index.len()
                && tools_with_index[tool_cursor].1.created_at < first_user.created_at
            {
                interleaved.push(tools_with_index[tool_cursor].1.clone());
                tool_cursor += 1;
            }
        }

        for idx in 0..users.len() {
            let user = users[idx].clone();
            let start_ts = user.created_at;
            let end_ts = users.get(idx + 1).map(|m| m.created_at).unwrap_or(i64::MAX);

            interleaved.push(user);

            while tool_cursor < tools_with_index.len() {
                let tool_msg = &tools_with_index[tool_cursor].1;
                if tool_msg.created_at < start_ts {
                    tool_cursor += 1;
                    continue;
                }
                if tool_msg.created_at >= end_ts {
                    break;
                }
                interleaved.push(tool_msg.clone());
                tool_cursor += 1;
            }
        }

        // Append any remaining tools (e.g., tools that landed after the last user).
        while tool_cursor < tools_with_index.len() {
            interleaved.push(tools_with_index[tool_cursor].1.clone());
            tool_cursor += 1;
        }

        interleaved
    }

    fn build_native_history_messages(
        scratchpad_entries: &[ScratchpadEntry],
    ) -> Vec<crate::types::Message> {
        let latest_execution_index = scratchpad_entries
            .iter()
            .rposition(|entry| matches!(entry.entry_type, ScratchpadEntryType::Execution(_)));

        scratchpad_entries
            .iter()
            .enumerate()
            .flat_map(|(idx, entry)| match &entry.entry_type {
                ScratchpadEntryType::PlanStep(_) => Vec::new(),
                ScratchpadEntryType::Execution(exec_entry) => {
                    let use_compaction = Some(idx) != latest_execution_index;
                    Self::execution_result_to_messages(&exec_entry.execution_result, use_compaction)
                }
                ScratchpadEntryType::Task(_) => Vec::new(),
                ScratchpadEntryType::Summary(summary) => {
                    // Render compaction summaries as system-like context messages
                    let mut msg = crate::types::Message::default();
                    msg.role = MessageRole::Assistant;
                    msg.created_at = entry.timestamp;
                    msg.parts = vec![Part::Text(format!(
                        "[Context summary — {} earlier entries compacted]: {}",
                        summary.entries_summarized, summary.summary_text
                    ))];
                    vec![msg]
                }
                ScratchpadEntryType::SkillContext(skill_ctx) => {
                    // Re-injected skill content — render as a system-like assistant message
                    let mut msg = crate::types::Message::default();
                    msg.role = MessageRole::Assistant;
                    msg.created_at = entry.timestamp;
                    msg.parts = vec![Part::Text(format!(
                        "[Skill context re-injected: {}]\n{}",
                        skill_ctx.skill_id, skill_ctx.content
                    ))];
                    vec![msg]
                }
            })
            .collect()
    }

    fn execution_result_to_messages(
        result: &ExecutionResult,
        compact: bool,
    ) -> Vec<crate::types::Message> {
        let history_result = if compact {
            result.compact_for_history()
        } else {
            result.clone()
        };
        let mut messages = Vec::new();
        let mut assistant_parts: Vec<Part> = Vec::new();
        let mut responded_tool_ids = HashSet::new();

        for part in history_result.parts.iter() {
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
                    message.created_at = history_result.timestamp;
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
                Part::File(file) => {
                    assistant_parts.push(Part::File(file.clone()));
                }
                Part::Artifact(artifact) => {
                    assistant_parts.push(Part::Artifact(artifact.clone()));
                }
            }
        }

        if !assistant_parts.is_empty() {
            // Orphan ToolCall (no matching ToolResult): DROP it.
            //
            // Old behaviour: stringify as `"Tool Call -> {name} with input:
            // {json}"`. That format is indistinguishable from a prompt
            // template the LLM is supposed to follow, so the model
            // faithfully re-emits the same call on the next turn — even if
            // the tool isn't in its tool set ("Tool 'run_skill' not found"
            // recursion). The text fallback was load-bearing for the
            // run_skill fan-out failure mode: a fork's LLM hallucinates one
            // run_skill call → tool not found → ExecutionResult with an
            // orphan ToolCall lands in scratchpad → next planning turn
            // stringifies it → LLM mimics → recursion.
            //
            // Real fix: drop the orphan part entirely. If after dropping
            // the assistant message has no parts, omit the assistant
            // message altogether.
            let assistant_parts: Vec<_> = assistant_parts
                .into_iter()
                .filter(|part| match part {
                    Part::ToolCall(tc) => responded_tool_ids.contains(&tc.tool_call_id),
                    _ => true,
                })
                .collect();
            if assistant_parts.is_empty() {
                return messages;
            }
            let mut assistant_message = crate::types::Message::default();
            assistant_message.role = MessageRole::Assistant;
            assistant_message.created_at = history_result.timestamp;
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

        // Lazily resolve {{> partial}} references from the DB.
        // Only fetches partials that are referenced AND not already registered (built-in).
        if let Some(ref store) = orchestrator.stores.prompt_template_store {
            let referenced = extract_partial_names(template);
            let known = prompt_registry.partial_names().await;
            let missing: Vec<String> = referenced
                .into_iter()
                .filter(|name| !known.contains(name))
                .collect();

            if !missing.is_empty() {
                match store.get_by_names(&missing).await {
                    Ok(templates) => {
                        for tpl in templates {
                            if let Err(e) = prompt_registry
                                .register_partial(tpl.name.clone(), tpl.template.clone())
                                .await
                            {
                                tracing::debug!("Failed to register partial '{}': {}", tpl.name, e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Failed to fetch partials from DB: {}", e);
                    }
                }
            }
        }

        let rendered_prompt = prompt_registry
            .render_template(template, template_data)
            .await?;
        Ok(rendered_prompt)
    } else {
        Ok(template.to_string())
    }
}

/// Extract partial names referenced in a template via `{{> name}}` syntax.
fn extract_partial_names(template: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = template;
    while let Some(pos) = rest.find("{{>") {
        rest = &rest[pos + 3..];
        let trimmed = rest.trim_start();
        // Partial name ends at `}}` or whitespace
        let end = trimmed
            .find(|c: char| c == '}' || c.is_whitespace())
            .unwrap_or(trimmed.len());
        let name = &trimmed[..end];
        if !name.is_empty() {
            names.push(name.to_string());
        }
    }
    names
}


#[cfg(test)]
#[path = "formatter_tests.rs"]
mod tests;
