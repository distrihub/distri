use distri_types::{AgentPlan, ExecutionResult, ExecutionStatus, Part};
use serde_json::Value;

use crate::{
    agent::{
        strategy::{
            execution::{ExecutionStrategy, MemoryStrategy},
            planning::PlanningStrategy,
        },
        types::{AgentHooks, MAX_ITERATIONS},
        AgentEventType, ExecutorContext,
    },
    types::Message,
    verbose_log, AgentError, StandardDefinition,
};
use std::sync::Arc;

/// Core agent execution loop using strategy pattern
#[derive(Clone, Debug)]
pub struct AgentLoop {
    pub agent_def: StandardDefinition,
    pub planner: Arc<dyn PlanningStrategy>,
    pub executor: Arc<dyn ExecutionStrategy>,
    pub memory: Arc<dyn MemoryStrategy>,
    pub hooks: Arc<dyn AgentHooks>,
}

impl AgentLoop {
    pub fn new(
        agent_def: StandardDefinition,
        planner: Arc<dyn PlanningStrategy>,
        executor: Arc<dyn ExecutionStrategy>,
        memory: Arc<dyn MemoryStrategy>,
        hooks: Arc<dyn AgentHooks>,
    ) -> Self {
        Self {
            agent_def,
            planner,
            executor,
            memory,
            hooks,
        }
    }

    pub async fn plan(
        &self,
        message: &Message,
        context: &Arc<ExecutorContext>,
    ) -> Result<AgentPlan, AgentError> {
        let mut msg = message.clone();
        self.hooks.on_plan_start(&mut msg, context.clone()).await?;
        context
            .emit(AgentEventType::PlanStarted { initial_plan: true })
            .await;
        let agent_plan = self.planner.plan(&msg, context.clone()).await?;

        // Store plan in context for easy access
        context.set_current_plan(Some(agent_plan.clone())).await;

        for step in &agent_plan.steps {
            context.store_plan_step(step).await;
        }

        context
            .emit(AgentEventType::PlanFinished {
                total_steps: agent_plan.steps.len(),
            })
            .await;
        self.hooks
            .on_plan_end(&mut msg, context.clone(), &agent_plan.steps)
            .await?;
        Ok(agent_plan)
    }

    pub async fn replan(
        &self,
        message: &Message,
        context: &Arc<ExecutorContext>,
        current_plan: &AgentPlan,
    ) -> Result<AgentPlan, AgentError> {
        let mut msg = message.clone();
        self.hooks.on_plan_start(&mut msg, context.clone()).await?;
        // Initial plan
        context
            .emit(AgentEventType::PlanStarted {
                initial_plan: false,
            })
            .await;
        let agent_plan = self
            .planner
            .replan(&msg, context.clone(), current_plan.clone())
            .await?;

        // Store updated plan in context
        context.set_current_plan(Some(agent_plan.clone())).await;

        context
            .emit(AgentEventType::PlanFinished {
                total_steps: agent_plan.steps.len(),
            })
            .await;
        self.hooks
            .on_plan_end(&mut msg, context.clone(), &agent_plan.steps)
            .await?;
        Ok(agent_plan)
    }
    pub async fn process_message(
        &self,
        message: &Message,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        let mut tool_parts = vec![];
        let mut non_tool_parts = vec![];

        if context.verbose {
            let line = "--------".repeat(3);
            tracing::info!("\n{}\n\n Processing message: \n{}\n", line, line);

            for part in &message.parts {
                if let Part::Text(text) = part {
                    tracing::info!("Text: {}", text);
                } else if let Part::ToolResult(tool_result) = part {
                    tracing::info!("ToolResult: {:#?}", tool_result);
                } else {
                    tracing::info!("Part: {}", part.type_name());
                }
            }

            tracing::info!("\n{}\n", line);
        }
        for part in &message.parts {
            if matches!(part, Part::ToolResult(_)) {
                tool_parts.push(part.clone());
            } else {
                non_tool_parts.push(part.clone());
            }
        }

        if !non_tool_parts.is_empty() {
            context.store_task(&non_tool_parts).await;
            let mut new_message = message.clone();
            new_message.parts = non_tool_parts;
            context.save_message(&new_message).await;
        }
        if !tool_parts.is_empty() {
            let step_id = context
                .get_current_step_id()
                .await
                .unwrap_or(uuid::Uuid::new_v4().to_string());
            tracing::debug!("Handling external tool result message");
            context
                .store_execution_result(&ExecutionResult {
                    step_id: step_id.clone(),
                    status: ExecutionStatus::Success,
                    parts: tool_parts.clone(),
                    timestamp: chrono::Utc::now().timestamp_millis(),
                    reason: None,
                })
                .await?;
        }

        Ok(())
    }

    pub async fn print_scratchpad(&self, context: &Arc<ExecutorContext>) {
        let logger = super::log::ModelLogger::new(Some(context.verbose));
        logger.log_scratchpad(context).await;
    }

    pub async fn run(
        &self,
        message: Message,
        context: Arc<ExecutorContext>,
    ) -> Result<Option<Value>, AgentError> {
        context.emit(AgentEventType::RunStarted {}).await;

        // Update task status to Running at the start of execution
        context
            .update_status(crate::types::TaskStatus::Running)
            .await;

        let mut execution_history = context.get_execution_history().await;

        // Save the initial message through orchestrator if available
        self.process_message(&message, context.clone()).await?;

        // Calculate context size after message is saved but before LLM calls
        if let Err(e) = context.calculate_context_size().await {
            tracing::warn!("Failed to calculate context size: {}", e);
        }

        // Auto-compact before planning if context is getting large
        if let Err(e) = context.evaluate_compaction().await {
            tracing::warn!("Failed to evaluate compaction: {}", e);
        }

        // Start with existing plan or None (will be planned in the loop)
        let mut current_plan = context.get_current_plan().await;

        self.memory.load_memory(context.clone()).await?;

        // Get configured max_steps from strategy
        let max_iterations = self.agent_def.max_iterations.unwrap_or(MAX_ITERATIONS);

        let mut error_iterations = 0;
        let mut step_index = 0;
        loop {
            if error_iterations > 2 {
                tracing::error!("Too many errors. Exiting...");
                break;
            }
            if current_plan.is_none()
                || step_index >= current_plan.as_ref().map(|p| p.steps.len()).unwrap_or(0)
            {
                match self.plan(&message, &context).await {
                    Ok(plan) => {
                        step_index = 0;
                        error_iterations = 0;
                        current_plan = Some(plan);

                        context.set_current_plan(current_plan.clone()).await;
                    }
                    Err(e) => {
                        tracing::error!("Planning failed: {}", e);
                        error_iterations = error_iterations + 1;

                        // Emit RunError event so UI can display the actual error
                        context
                            .emit(AgentEventType::RunError {
                                message: format!("Planning failed: {}", e),
                                code: Some("PLANNING_ERROR".to_string()),
                            })
                            .await;

                        let result = ExecutionResult {
                            step_id: uuid::Uuid::new_v4().to_string(),
                            status: ExecutionStatus::Failed,
                            parts: vec![],
                            timestamp: chrono::Utc::now().timestamp_millis(),
                            reason: Some(format!("Planning failed: {}", e)),
                        };
                        context.store_execution_result(&result).await?;
                        continue;
                    }
                }
            }

            let current_plan_ref = current_plan.as_ref().unwrap();
            verbose_log!(
                context.verbose,
                "Step index: {}, total steps: {}, max_steps: {}",
                step_index,
                current_plan_ref.steps.len(),
                max_iterations
            );
            // Print scratchpad in verbose mode after each step
            if context.verbose {
                self.print_scratchpad(&context).await;
            }

            // If we've reached max steps, check if the last step included a final tool call
            // If so, allow completion; otherwise enforce the limit
            if step_index >= max_iterations {
                let has_final_call = context.get_final_result().await.is_some();
                if has_final_call {
                    verbose_log!(
                        context.verbose,
                        "Max iterations ({}) reached but final tool was called, allowing completion",
                        max_iterations
                    );
                    break;
                } else {
                    verbose_log!(
                        context.verbose,
                        "Max iterations ({}) reached (executed {} iterations), stopping execution",
                        max_iterations,
                        step_index
                    );
                    break;
                }
            }

            let step = current_plan_ref
                .steps
                .get(step_index)
                .ok_or(AgentError::Planning(format!(
                    "Plan exhausted: step index: {}, total steps: {}",
                    step_index,
                    current_plan_ref.steps.len()
                )));
            if let Err(e) = step {
                tracing::error!("ERROR: Needs to replan: {}", e);
                context.set_current_plan(None).await;
                current_plan = None;
                step_index = 0;
                continue;
            }
            let step = step.unwrap();
            let step_id = step.id.clone();

            // Set the current step id for the step
            context.set_current_step_id(Some(step_id.clone())).await;

            self.hooks.on_step_start(&step).await?;
            context
                .emit(AgentEventType::StepStarted {
                    step_id: step_id.clone(),
                    step_index,
                })
                .await;

            let result = match self
                .executor
                .execute_step_stream(&step, context.clone())
                .await
            {
                Ok(result) => result,
                Err(e) => {
                    // Emit RunError event for critical failures like LLM errors
                    tracing::error!("Step execution failed: {}", e);
                    context
                        .update_status(crate::types::TaskStatus::Failed)
                        .await;
                    context
                        .emit(AgentEventType::RunError {
                            message: format!("Step execution failed: {}", e),
                            code: Some("EXECUTION_ERROR".to_string()),
                        })
                        .await;

                    return Err(e);
                }
            };

            verbose_log!(
                context.verbose,
                "Execution result for agent_id: {},  task_id: {},  result: {:?}",
                context.agent_id,
                context.task_id,
                result.as_observation()
            );

            if result.is_input_required() {
                verbose_log!(context.verbose, "Input required, stopping execution");
                context
                    .update_status(crate::types::TaskStatus::InputRequired)
                    .await;
                break;
            }
            // Store both the plan step and execution result in scratchpad store
            context.store_execution_result(&result).await?;

            // Increment iteration count per executed step (not per plan)
            context.increment_iteration().await;

            // Store the execution result for agent loop history
            execution_history.push(result.clone());

            // Call hooks and memory storage
            self.hooks
                .on_step_end(context.clone(), &step, &result)
                .await?;
            self.memory.store_step_result(&step, &result).await?;

            verbose_log!(
                context.verbose,
                "Step completed: agent_id: {}, task_id: {}, step_id: {}, success: {}",
                context.agent_id,
                context.task_id,
                step_id,
                result.is_success()
            );
            context
                .emit(AgentEventType::StepCompleted {
                    step_id: step_id.clone(),
                    success: result.is_success(),
                    context_budget: Some(context.get_usage().await.context_budget.clone()),
                })
                .await;

            // Periodic replan
            if self.planner.needs_replanning(&execution_history) {
                let agent_plan = match self
                    .replan(&message, &context, current_plan.as_ref().unwrap())
                    .await
                {
                    Ok(plan) => plan,
                    Err(e) => {
                        // Emit RunError event for periodic replanning failures like LLM errors
                        tracing::error!("Periodic replanning failed: {}", e);
                        context
                            .update_status(crate::types::TaskStatus::Failed)
                            .await;
                        context
                            .emit(AgentEventType::RunError {
                                message: format!("Periodic replanning failed: {}", e),
                                code: Some("PERIODIC_REPLANNING_ERROR".to_string()),
                            })
                            .await;

                        return Err(e);
                    }
                };
                current_plan = Some(agent_plan.clone());
                context.set_current_plan(Some(agent_plan)).await;
                step_index = 0;

                continue;
            }
            // Check if we should continue by calling the executor's should_continue method with the last result

            let should_continue = self
                .executor
                .should_continue(
                    &current_plan.as_ref().unwrap().steps,
                    step_index.saturating_sub(1),
                    context.clone(),
                )
                .await;

            if !should_continue {
                // Subagent-based reflection (if enabled)
                if self.agent_def.is_reflection_enabled() {
                    // Use a simple heuristic: if we have more than 5 iterations or already have reflection results, skip
                    let reflection_completed = execution_history
                        .iter()
                        .any(|result| result.step_id == "reflection");

                    if !reflection_completed {
                        verbose_log!(
                            context.verbose,
                            "agent_id: {}, task_id: {}, 🤔 Starting reflection analysis (reflection enabled)",
                            context.agent_id,
                            context.task_id
                        );
                        if let Ok(true) = self.reflect(&message, &context).await {
                            continue;
                        }
                    }
                }
                verbose_log!(
                    context.verbose,
                    "agent_id: {}, task_id: {}, Executor decided not to continue, stopping execution",
                    context.agent_id,
                    context.task_id
                );
                break;
            }
            step_index += 1;
        }

        // Reload execution history from context to include any results stored during planning failures
        execution_history = context.get_execution_history().await;

        let failed_steps = execution_history
            .iter()
            .filter(|result| result.is_failed())
            .count();
        let total_steps = execution_history.len();

        let validation_result = self.validate_completion(&execution_history, &context).await;
        let final_success = validation_result.is_ok();

        let last_result = execution_history.last();
        if let Some(last_result) = last_result {
            // Update task status based on completion result
            let final_status = last_result.status.clone().into();
            context.update_status(final_status).await;
        }
        let final_result = context.get_final_result().await;
        verbose_log!(
            context.verbose,
            "Run finished: agent_id: {}, task_id: {}, success: {}, total_steps: {}, failed_steps: {}, final_result: {:?}",
            context.agent_id,
            context.task_id,
            final_success,
            total_steps,
            failed_steps,
            final_result
        );

        // Get usage info from context
        let usage_info = context.get_usage().await;
        let cost_usd = usage_info.model.as_ref().and_then(|m| {
            estimate_cost(
                m,
                usage_info.input_tokens,
                usage_info.output_tokens,
                usage_info.cached_tokens,
            )
        });
        let run_usage = distri_types::RunUsage {
            total_tokens: usage_info.tokens,
            input_tokens: usage_info.input_tokens,
            output_tokens: usage_info.output_tokens,
            cached_tokens: usage_info.cached_tokens,
            estimated_tokens: usage_info.context_size.total_estimated_tokens as u32,
            model: usage_info.model.clone(),
            cost_usd,
        };

        context
            .emit(AgentEventType::RunFinished {
                success: final_success,
                total_steps,
                failed_steps,
                usage: Some(run_usage),
                context_budget: Some(context.get_usage().await.context_budget.clone()),
            })
            .await;
        // Return validation error if completion was invalid (to maintain existing behavior)
        if let Err(e) = validation_result {
            // Emit RunError event so UI can display the validation error
            context
                .emit(AgentEventType::RunError {
                    message: e.to_string(),
                    code: Some("VALIDATION_ERROR".to_string()),
                })
                .await;
            return Err(e);
        }

        Ok(final_result)
    }

    /// Run reflection subagent to analyze execution and store results.
    /// The reflection agent uses the `reflect` tool call to signal its decision,
    /// rather than relying on "Should Continue" string matching in text output.
    /// Returns true if reflection recommends retrying execution.
    async fn reflect(
        &self,
        message: &Message,
        context: &Arc<ExecutorContext>,
    ) -> Result<bool, AgentError> {
        let orchestrator = context
            .orchestrator
            .as_ref()
            .ok_or(AgentError::NotFound("Orchestrator not found".to_string()))?;
        let task_text = message
            .as_text()
            .unwrap_or_else(|| "Unknown task".to_string());

        let execution_history = context
            .format_agent_scratchpad(None)
            .await
            .unwrap_or_default();
        let final_result = context.get_final_result().await;
        let has_final = final_result.is_some();

        let task_description = if has_final {
            format!(
                "Analyze the execution history for task: {}\n\nIMPORTANT: A final result was provided. Please evaluate its quality and suggest revisions if needed.\nFinal result: {:?}",
                task_text,
                final_result.unwrap_or_default()
            )
        } else {
            format!(
                "Analyze the execution history for task: {}\n\nNOTE: No final result was provided yet.",
                task_text
            )
        };

        let reflection_result = crate::agent::reflection::run_reflection_agent(
            orchestrator,
            context.clone(),
            &task_description,
            &execution_history,
        )
        .await?;

        // The reflection agent uses the `reflect` tool which stores its structured
        // result as the final result. Extract `should_continue` from the tool call output.
        let should_retry = reflection_result
            .final_result
            .as_ref()
            .and_then(|v| v.get("should_continue"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let reflection_text = reflection_result.content;

        if should_retry {
            context
                .store_execution_result(&ExecutionResult {
                    step_id: "reflection".to_string(),
                    status: ExecutionStatus::Success,
                    parts: vec![Part::Text(reflection_text)],
                    timestamp: chrono::Utc::now().timestamp_millis(),
                    reason: Some("Reflection analysis completed".to_string()),
                })
                .await?;

            context.set_final_result(None).await;
            return Ok(true);
        }
        Ok(false)
    }

    /// Validates that the agent properly completed the task
    async fn validate_completion(
        &self,
        history: &[ExecutionResult],
        context: &Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        if let Some(crate::types::TaskStatus::InputRequired) = context.get_status().await {
            return Ok(());
        }
        if history.is_empty() {
            return Err(AgentError::Planning(
                "Agent completed without executing any steps".to_string(),
            ));
        }

        // Check if any execution result is marked as final
        let has_final_call = context.get_final_result().await.is_some();

        // If no final tool was called, check if all steps completed successfully
        if !has_final_call {
            let all_successful = history.iter().all(|result| result.is_success());
            let has_failures = history.iter().any(|result| result.is_failed());

            if has_failures {
                return Err(AgentError::Planning(
                    "Agent execution completed with failures and no final result provided. Please use the 'final' tool to provide the complete answer.".to_string(),
                ));
            }

            if !all_successful {
                return Err(AgentError::Planning(
                    "Agent execution completed unsuccessfully without calling the 'final' tool. Please complete the task and provide a final result.".to_string(),
                ));
            }

            // Even if successful, warn about missing final call for non-code execution modes
            tracing::warn!(
                "Agent completed without calling 'final' tool. This may indicate incomplete task execution."
            );
        }

        Ok(())
    }
}

/// Model pricing entry loaded from model_pricing.json
#[derive(Debug, Clone, serde::Deserialize)]
struct ModelPricing {
    input: f64,
    output: f64,
    #[serde(default)]
    cached_input: f64,
}

#[derive(Debug, serde::Deserialize)]
struct PricingFile {
    models: std::collections::HashMap<String, ModelPricing>,
}

/// Load pricing from embedded JSON file, cached in a static.
fn get_model_pricing() -> &'static std::collections::HashMap<String, ModelPricing> {
    use std::sync::OnceLock;
    static PRICING: OnceLock<std::collections::HashMap<String, ModelPricing>> = OnceLock::new();
    PRICING.get_or_init(|| {
        let json = include_str!("../../model_pricing.json");
        let file: PricingFile =
            serde_json::from_str(json).expect("Failed to parse model_pricing.json");
        file.models
    })
}

/// Estimate cost in USD based on model name, token counts, and cached tokens.
/// Prices loaded from model_pricing.json (per 1M tokens).
/// Cached tokens are charged at the discounted cached_input rate instead of full input rate.
fn estimate_cost(
    model: &str,
    input_tokens: u32,
    output_tokens: u32,
    cached_tokens: u32,
) -> Option<f64> {
    let pricing = get_model_pricing();

    // Try exact match first, then longest substring match
    let entry = pricing.get(model).or_else(|| {
        pricing
            .iter()
            .filter(|(key, _)| model.contains(key.as_str()))
            .max_by_key(|(key, _)| key.len())
            .map(|(_, v)| v)
    })?;

    // Non-cached input tokens = total input - cached
    let non_cached_input = if cached_tokens > input_tokens {
        0
    } else {
        input_tokens - cached_tokens
    };

    let cost = (non_cached_input as f64 * entry.input
        + cached_tokens as f64 * entry.cached_input
        + output_tokens as f64 * entry.output)
        / 1_000_000.0;

    Some((cost * 10000.0).round() / 10000.0) // Round to 4 decimal places
}

#[cfg(test)]
mod cost_tests {
    use super::*;

    #[test]
    fn pricing_json_loads() {
        let pricing = get_model_pricing();
        assert!(
            pricing.len() >= 10,
            "Expected at least 10 models in pricing"
        );
        assert!(pricing.contains_key("gpt-5.1"));
        assert!(pricing.contains_key("claude-sonnet-4"));
    }

    #[test]
    fn exact_model_match() {
        let cost = estimate_cost("gpt-5.1", 1_000_000, 0, 0).unwrap();
        assert_eq!(cost, 2.0); // $2/M input tokens
    }

    #[test]
    fn fuzzy_model_match_prefers_longest() {
        // "gpt-4o-mini-2024-07-18" should match "gpt-4o-mini" not "gpt-4o"
        let cost = estimate_cost("gpt-4o-mini-2024-07-18", 1_000_000, 0, 0).unwrap();
        assert_eq!(cost, 0.15); // gpt-4o-mini input price, not gpt-4o's $2.50
    }

    #[test]
    fn cached_tokens_use_discount() {
        // 500k cached + 500k non-cached input for claude-sonnet-4
        // Non-cached: 500k * $3/M = $1.50
        // Cached: 500k * $0.375/M = $0.1875
        let cost = estimate_cost("claude-sonnet-4", 1_000_000, 0, 500_000).unwrap();
        let expected: f64 = (500_000.0 * 3.0 + 500_000.0 * 0.375) / 1_000_000.0;
        assert_eq!(cost, (expected * 10000.0).round() / 10000.0);
    }

    #[test]
    fn unknown_model_returns_none() {
        assert!(estimate_cost("totally-unknown-model", 1000, 1000, 0).is_none());
    }

    #[test]
    fn output_tokens_costed() {
        let cost = estimate_cost("gpt-5.1", 0, 1_000_000, 0).unwrap();
        assert_eq!(cost, 8.0); // $8/M output tokens
    }
}
