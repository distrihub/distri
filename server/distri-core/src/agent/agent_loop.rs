use distri_types::{AgentPlan, ExecutionResult, ExecutionStatus, Part};
use serde_json::Value;

use crate::durable::SegmentKind;
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

/// What resuming a parked step found.
enum FoldOutcome {
    /// No journaled step.
    Nothing,
    /// The step is stored with every result; `end_turn` when a result asks
    /// the turn to end (`should_continue: false`).
    Folded { end_turn: bool },
    /// Some calls are still pending (a crashed segment resumed early).
    StillWaiting(distri_types::WaitingOn),
}

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
            // Re-indexes parts_metadata so save/developer flags follow their parts.
            let new_message = message.retain_parts(|_, part| !matches!(part, Part::ToolResult(_)));
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

        let durable = context.durable.clone();
        let mut end_turn_after_fold = false;
        match durable.as_ref().map(|d| d.kind) {
            Some(SegmentKind::Resume) | Some(SegmentKind::ResumeWithMessage) => {
                // A resumed segment: fold the parked step's results into one
                // tool-results entry (S2); the triggering message is already
                // saved, so it is not processed again.
                match self.fold_pending_step(&context).await? {
                    FoldOutcome::StillWaiting(waiting) => {
                        return self.park(&context, waiting).await;
                    }
                    FoldOutcome::Folded { end_turn } => end_turn_after_fold = end_turn,
                    FoldOutcome::Nothing => {}
                }
                if durable.as_ref().map(|d| d.kind) == Some(SegmentKind::ResumeWithMessage) {
                    self.process_message(&message, context.clone()).await?;
                }
            }
            _ => {
                // Save the initial message through orchestrator if available
                self.process_message(&message, context.clone()).await?;
            }
        }
        let mut execution_history = context.get_execution_history().await;

        // Calculate context size after message is saved but before LLM calls
        if let Err(e) = context.calculate_context_size().await {
            tracing::warn!("Failed to calculate context size: {}", e);
        }

        // Stash a Tier 2 summary LLM executor on the context so
        // `evaluate_compaction` can actually summarize (not just trim)
        // when the size manager picks Summarize-tier. Strategies that
        // don't override `build_summary_executor` return None — in that
        // case Tier 2 falls back to mechanical trim.
        if let Some(executor) = self.planner.build_summary_executor(context.clone()).await {
            *context.summary_executor.write().await = Some(executor);
        }

        // Auto-compact before planning if context is getting large.
        // evaluate_compaction() handles persistence + skill re-injection.
        if let Err(e) = context.evaluate_compaction().await {
            tracing::warn!("Failed to evaluate compaction: {}", e);
        }

        // Emit initial budget update so CLI shows context state
        context.emit_budget_update().await;

        // Start with existing plan or None (will be planned in the loop)
        let mut current_plan = context.get_current_plan().await;

        self.memory.load_memory(context.clone()).await?;

        // Get configured max_steps from strategy
        let max_iterations = self.agent_def.max_iterations.unwrap_or(MAX_ITERATIONS);

        let mut error_iterations = 0;
        let mut last_planning_error: Option<AgentError> = None;
        let mut step_index = 0;
        let mut plan_cycle: usize = 0;
        loop {
            if end_turn_after_fold {
                break;
            }
            // Snapshot at the top of each iteration so per-step deltas cover the
            // planning LLM call + any tools executed before the next plan cycle.
            context.snapshot_step_start().await;

            // Check cancellation signal between iterations for cooperative abort
            if let Some(ref signal) = context.cancellation_signal {
                if signal.is_cancelled().await {
                    tracing::info!(
                        "Agent loop cancelled for task_id: {}, agent_id: {}",
                        context.task_id,
                        context.agent_id
                    );
                    context
                        .update_status(crate::types::TaskStatus::Canceled)
                        .await;
                    context
                        .emit(AgentEventType::RunError {
                            message: "Task cancelled".to_string(),
                            code: Some("CANCELLED".to_string()),
                            usage: Some(context.get_step_usage().await),
                        })
                        .await;
                    return Err(AgentError::Canceled);
                }
            }

            // Drain mailbox for inter-agent messages and inject into context
            if let Some(ref mailbox) = context.mailbox {
                let messages = mailbox.lock().await.drain().await;
                for msg in messages {
                    tracing::info!(
                        "Agent {} received inter-agent message from {}",
                        context.agent_id,
                        msg.from
                    );
                    let injected = Message::user(
                        format!(
                            "<agent-message from=\"{}\">\n{}\n</agent-message>",
                            msg.from, msg.content
                        ),
                        None,
                    );
                    context.save_message(&injected).await;
                }
            }

            if error_iterations > 2 {
                tracing::error!("Too many errors. Exiting...");
                // Only now is the planning failure the run's end: one
                // terminal PLANNING_ERROR (retries above emit none, S12).
                if let Some(e) = last_planning_error.take() {
                    context
                        .update_status(crate::types::TaskStatus::Failed)
                        .await;
                    context
                        .emit(AgentEventType::RunError {
                            message: format!("Planning failed: {}", e),
                            code: Some("PLANNING_ERROR".to_string()),
                            usage: Some(context.get_step_usage().await),
                        })
                        .await;
                    return Err(e);
                }
                break;
            }

            // Durable runs count steps across segments, so a resumed task
            // cannot loop past its limit.
            if durable.is_some()
                && context.get_usage().await.current_iteration >= max_iterations
                && context.get_final_result().await.is_none()
            {
                verbose_log!(
                    context.verbose,
                    "Max iterations ({}) reached across segments, stopping",
                    max_iterations
                );
                break;
            }

            // Early termination if anything set a final_result between
            // iterations (e.g. the planner auto-finalized after MAX_RETRIES of
            // text-only LLM responses). Without this check, an empty plan
            // would trigger replanning and the same pattern would repeat.
            if context.get_final_result().await.is_some() {
                verbose_log!(
                    context.verbose,
                    "Final result already set before planning; stopping loop"
                );
                break;
            }

            // Emit StepStarted BEFORE planning so the plan span nests under the step span
            // in the OTel trace hierarchy. Use the global iteration count as the step index
            // so it reflects overall progress rather than local-plan position.
            let global_step_index = context.get_usage().await.current_iteration;
            let iteration_step_id = uuid::Uuid::new_v4().to_string();
            context
                .set_current_step_id(Some(iteration_step_id.clone()))
                .await;
            context
                .emit(AgentEventType::StepStarted {
                    step_id: iteration_step_id.clone(),
                    step_index: global_step_index,
                })
                .await;

            if current_plan.is_none()
                || step_index >= current_plan.as_ref().map(|p| p.steps.len()).unwrap_or(0)
            {
                plan_cycle += 1;
                context
                    .emit_verbose(format!(
                        "[plan] cycle {} (step_index {})",
                        plan_cycle, step_index
                    ))
                    .await;
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

                        // A retried planning error is not the run's end: no
                        // terminal RunError here (it closed the client stream
                        // while the loop retried). The step closes as failed;
                        // exhausting the retries emits the one terminal error.
                        context
                            .emit_verbose(format!("Planning failed (will retry): {}", e))
                            .await;

                        let result = ExecutionResult {
                            step_id: iteration_step_id.clone(),
                            status: ExecutionStatus::Failed,
                            parts: vec![],
                            timestamp: chrono::Utc::now().timestamp_millis(),
                            reason: Some(format!("Planning failed: {}", e)),
                        };
                        context.store_execution_result(&result).await?;
                        last_planning_error = Some(e);
                        // Close the step span we opened before continuing
                        context
                            .emit(AgentEventType::StepCompleted {
                                step_id: iteration_step_id.clone(),
                                success: false,
                                context_budget: None,
                                usage: Some(context.get_step_usage().await),
                            })
                            .await;
                        continue;
                    }
                }
            }

            // A plan with no steps while the run already has its answer: the
            // planner auto-finalized a plain-text reply (distri #163). Close the
            // step as done and stop, rather than treating the empty plan as
            // exhausted (whose error log marks the run's trace span ERROR).
            if current_plan
                .as_ref()
                .is_some_and(|plan| plan.steps.is_empty())
                && context.get_final_result().await.is_some()
            {
                context
                    .emit(AgentEventType::StepCompleted {
                        step_id: iteration_step_id.clone(),
                        success: true,
                        context_budget: Some(context.get_usage().await.context_budget.clone()),
                        usage: Some(context.get_step_usage().await),
                    })
                    .await;
                break;
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
                // Close the step span we opened before breaking
                context
                    .emit(AgentEventType::StepCompleted {
                        step_id: iteration_step_id.clone(),
                        success: has_final_call,
                        context_budget: None,
                        usage: Some(context.get_step_usage().await),
                    })
                    .await;
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
                // Close the step span before retrying
                context
                    .emit(AgentEventType::StepCompleted {
                        step_id: iteration_step_id.clone(),
                        success: false,
                        context_budget: None,
                        usage: None,
                    })
                    .await;
                continue;
            }
            let step = step.unwrap();

            self.hooks.on_step_start(&step).await?;

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
                            usage: Some(context.get_step_usage().await),
                        })
                        .await;
                    // Close the step span before returning
                    context
                        .emit(AgentEventType::StepCompleted {
                            step_id: iteration_step_id.clone(),
                            success: false,
                            context_budget: None,
                            usage: Some(context.get_step_usage().await),
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
                let pending = match &durable {
                    Some(d) => d
                        .state
                        .lock()
                        .await
                        .pending_step
                        .as_ref()
                        .map(|p| p.pending.clone())
                        .unwrap_or_default(),
                    None => Vec::new(),
                };
                if !pending.is_empty() {
                    // Durable: the step's calls wait outside the worker. The
                    // step stays journaled (folded on resume); park.
                    context
                        .emit(AgentEventType::StepCompleted {
                            step_id: iteration_step_id.clone(),
                            success: true,
                            context_budget: Some(context.get_usage().await.context_budget.clone()),
                            usage: Some(context.get_step_usage().await),
                        })
                        .await;
                    let deadline = self.earliest_deadline(&context, &pending).await;
                    return self
                        .park(&context, distri_types::WaitingOn { pending, deadline })
                        .await;
                }
                context
                    .update_status(crate::types::TaskStatus::InputRequired)
                    .await;
                // Keep the step's tool-call parts (and any inline results):
                // breaking before storing lost them (S11).
                context.store_execution_result(&result).await?;
                context
                    .emit(AgentEventType::StepCompleted {
                        step_id: iteration_step_id.clone(),
                        success: true,
                        context_budget: Some(context.get_usage().await.context_budget.clone()),
                        usage: Some(context.get_step_usage().await),
                    })
                    .await;
                break;
            }
            // Durable: the step is stored whole below; drop its journal.
            if let Some(d) = &durable {
                d.state.lock().await.pending_step = None;
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
                iteration_step_id,
                result.is_success()
            );
            context
                .emit(AgentEventType::StepCompleted {
                    step_id: iteration_step_id.clone(),
                    success: result.is_success(),
                    context_budget: Some(context.get_usage().await.context_budget.clone()),
                    usage: Some(context.get_step_usage().await),
                })
                .await;

            // Emit budget update after each step so CLI tracks context growth
            context.emit_budget_update().await;

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
                                usage: Some(context.get_step_usage().await),
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
                            "agent_id: {}, task_id: {}, Starting reflection analysis (reflection enabled)",
                            context.agent_id,
                            context.task_id
                        );
                        context.emit(AgentEventType::ReflectStarted {}).await;
                        let should_retry = self.reflect(&message, &context).await;
                        let (retry, reason) = match should_retry {
                            Ok(true) => (true, None),
                            Ok(false) => (false, None),
                            Err(ref e) => (false, Some(e.to_string())),
                        };
                        context
                            .emit(AgentEventType::ReflectFinished {
                                should_retry: retry,
                                reason,
                            })
                            .await;
                        if let Ok(true) = should_retry {
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
        let input_required = matches!(
            context.get_status().await,
            Some(crate::types::TaskStatus::InputRequired)
        );
        if input_required {
            // Waiting on input is not completion: a later step's status must
            // not overwrite it (S11).
        } else if let Some(last_result) = last_result {
            // Update task status based on completion result
            let final_status = last_result.status.clone().into();
            context.update_status(final_status).await;
        } else if final_success
            && context.get_final_result().await.is_some()
            && !matches!(
                context.get_status().await,
                Some(crate::types::TaskStatus::InputRequired)
            )
        {
            // Zero steps but an answer (a plain-text reply the planner
            // auto-finalized): there is no step result to take the status from.
            context
                .update_status(crate::types::TaskStatus::Completed)
                .await;
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

        // Durable: the final answer is saved before the terminal event, so a
        // stream that ends on it reads the answer from the store (B3) —
        // whichever segment and server produced it.
        if durable.is_some() {
            if let Some(text) = final_result
                .as_ref()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .filter(|t| !t.is_empty())
            {
                context
                    .save_message(&Message::final_answer(&context.task_id, text))
                    .await;
            }
        }

        context
            .emit(AgentEventType::RunFinished {
                success: final_success,
                total_steps,
                failed_steps,
                usage: Some(context.get_total_usage().await),
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
                    usage: Some(context.get_step_usage().await),
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

    /// End the segment waiting on `waiting`: InputRequired, one `RunParked`
    /// (not terminal), no RunFinished. The worker parks the claim.
    async fn park(
        &self,
        context: &Arc<ExecutorContext>,
        waiting: distri_types::WaitingOn,
    ) -> Result<Option<Value>, AgentError> {
        context
            .update_status(crate::types::TaskStatus::InputRequired)
            .await;
        let attempt = context
            .durable
            .as_ref()
            .map(|d| d.claim.attempt)
            .unwrap_or(0);
        context
            .emit(AgentEventType::RunParked {
                task_id: context.task_id.clone(),
                pending: waiting.pending.clone(),
                deadline: waiting.deadline,
                attempt,
            })
            .await;
        if let Some(d) = &context.durable {
            *d.parked.lock().await = Some(waiting);
        }
        Ok(None)
    }

    async fn earliest_deadline(
        &self,
        context: &Arc<ExecutorContext>,
        pending: &[String],
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        let d = context.durable.as_ref()?;
        let mut earliest = None;
        for id in pending {
            if let Ok(Some(call)) = d.runtime.pending_calls.get(id).await {
                if let Some(dl) = call.deadline {
                    earliest =
                        Some(earliest.map_or(dl, |e: chrono::DateTime<chrono::Utc>| e.min(dl)));
                }
            }
        }
        earliest
    }

    /// Fold the journaled step of a parked (or crashed) segment into ONE
    /// tool-results entry, one result per tool call, in call order (S2):
    /// inline results as recorded, pending calls as answered (completed,
    /// expired, canceled, interrupted), an inline call cut off by a crash
    /// re-run when replay-safe, else `interrupted` (R4).
    async fn fold_pending_step(
        &self,
        context: &Arc<ExecutorContext>,
    ) -> Result<FoldOutcome, AgentError> {
        let Some(durable) = context.durable.clone() else {
            return Ok(FoldOutcome::Nothing);
        };
        let Some(step) = durable.state.lock().await.pending_step.clone() else {
            return Ok(FoldOutcome::Nothing);
        };
        let store = durable.runtime.pending_calls.clone();
        let calls: std::collections::HashMap<String, distri_types::PendingCall> = store
            .list_for_task(&context.task_id)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?
            .into_iter()
            .map(|c| (c.id.clone(), c))
            .collect();

        let still_pending: Vec<String> = step
            .pending
            .iter()
            .filter(|id| {
                calls
                    .get(*id)
                    .is_some_and(|c| c.status == distri_types::PendingStatus::Pending)
            })
            .cloned()
            .collect();
        if !still_pending.is_empty() {
            let deadline = self.earliest_deadline(context, &still_pending).await;
            return Ok(FoldOutcome::StillWaiting(distri_types::WaitingOn {
                pending: still_pending,
                deadline,
            }));
        }

        let tools = context.get_tools().await;
        let mut results = Vec::with_capacity(step.tool_calls.len());
        let mut fresh = Vec::new();
        for call in &step.tool_calls {
            let id = &call.tool_call_id;
            let response = if let Some(r) = step.results.iter().find(|r| &r.tool_call_id == id) {
                r.clone()
            } else if step.pending.contains(id) {
                let result = calls
                    .get(id)
                    .and_then(|c| c.result.clone())
                    .unwrap_or_else(distri_types::CallResult::interrupted);
                let r = result.to_tool_response(id, &call.tool_name);
                fresh.push(r.clone());
                r
            } else if step.replay_safe.contains(id) {
                let tool = crate::agent::tool_lookup::find_tool_by_name(&tools, &call.tool_name);
                let r = match tool {
                    Some(tool) => {
                        let (parts, _) =
                            crate::agent::strategy::execution::default::run_inline_tool(
                                tool.as_ref(),
                                call,
                                context,
                            )
                            .await;
                        crate::types::ToolResponse::from_parts(
                            id.clone(),
                            call.tool_name.clone(),
                            parts,
                        )
                    }
                    None => distri_types::CallResult::interrupted()
                        .to_tool_response(id, &call.tool_name),
                };
                fresh.push(r.clone());
                r
            } else {
                // In flight when the worker died and not replay-safe: never
                // run twice. A synchronous sub-agent lands here too.
                let r =
                    distri_types::CallResult::interrupted().to_tool_response(id, &call.tool_name);
                fresh.push(r.clone());
                r
            };
            results.push(response);
        }

        let mut parts = step.assistant_parts.clone();
        parts.extend(results.into_iter().map(Part::ToolResult));
        let end_turn = crate::agent::strategy::execution::default::parts_request_turn_end(&parts);
        let result = ExecutionResult {
            step_id: step.step_id.clone(),
            status: ExecutionStatus::Success,
            parts,
            timestamp: chrono::Utc::now().timestamp_millis(),
            reason: None,
        };
        context.store_execution_result(&result).await?;
        if !fresh.is_empty() {
            context
                .emit(AgentEventType::ToolResults {
                    step_id: step.step_id.clone(),
                    parent_message_id: None,
                    results: fresh,
                })
                .await;
        }
        context.increment_iteration().await;
        durable.state.lock().await.pending_step = None;
        durable.sync_from_context(context).await;
        Ok(FoldOutcome::Folded { end_turn })
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
        // Check if any execution result is marked as final
        let has_final_call = context.get_final_result().await.is_some();

        // A run with no executed steps is complete when it has an answer: the
        // planner auto-finalizes a plain-text LLM reply into the final result
        // and returns a plan with zero steps (distri #163).
        if history.is_empty() {
            if has_final_call {
                return Ok(());
            }
            return Err(AgentError::Planning(
                "Agent completed without executing any steps".to_string(),
            ));
        }

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
