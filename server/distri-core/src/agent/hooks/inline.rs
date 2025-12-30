use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

use crate::agent::context::{ExecutorContext, HookPromptState};
use crate::agent::types::AgentHooks;
use crate::AgentError;
use browsr_client::{default_transport as browsr_default_transport, BrowsrClient};
use distri::{HookContext, HookMutation};
use distri_types::{AgentEventType, ExecutionResult, InlineHookRequest, Message, PlanStep};

/// Apply a hook mutation to the message context.
pub async fn apply_mutation(
    _message: &mut crate::types::Message,
    context: &Arc<ExecutorContext>,
    mutation: HookMutation,
) {
    if !mutation.dynamic_values.is_empty() {
        let mut state = HookPromptState::default();
        state.dynamic_values = mutation.dynamic_values;
        context.merge_hook_prompt_state(state).await;
    }
}

/// Inline hook dispatcher: emits an event and waits for a mutation to be
/// delivered via POST /event/hooks (or stdout transport completion).
#[derive(Clone, Debug)]
pub struct InlineHook {
    timeout: Duration,
}

impl Default for InlineHook {
    fn default() -> Self {
        Self {
            timeout: Duration::from_millis(30_000),
        }
    }
}

impl InlineHook {
    pub fn new(timeout_ms: Option<u64>) -> Self {
        Self {
            timeout: Duration::from_millis(timeout_ms.unwrap_or(30_000)),
        }
    }

    async fn request(
        &self,
        hook: &str,
        message: Option<Message>,
        plan: Option<Vec<PlanStep>>,
        result: Option<ExecutionResult>,
        context: Arc<ExecutorContext>,
    ) -> distri_types::HookMutation {
        // If a local inline hook handler is registered for this agent+hook, use it immediately.
        let request = InlineHookRequest {
            hook: hook.to_string(),
            hook_id: String::new(), // overwritten if we fall back to async path
            context: HookContext::from_parts(
                &context.agent_id,
                &context.thread_id,
                &context.task_id,
                &context.run_id,
            ),
            timeout_ms: self.timeout.as_millis() as u64,
            fire_and_forget: context
                .tool_metadata
                .as_ref()
                .and_then(|m| m.get("hook_behavior"))
                .and_then(|v| v.get(hook))
                .and_then(|b| b.as_str())
                .map(|s| s == "fire_and_forget")
                .unwrap_or(false),
            message: message.clone(),
            plan: plan.clone(),
            result: result.clone(),
            observation: None,
            sequence_id: None,
        };

        // Fire-and-forget: execute hook handler if registered (no return value)
        if let Some(registry) = context.hook_registry.read().await.as_ref() {
            registry.try_handle(&context.agent_id, &request).await;
            // Since hooks are fire-and-forget, we continue to the fallback logic below
        }

        // Best-effort inline enrichment for the browsr agent without external callbacks.
        if let Some(mutation) = maybe_enrich_browsr(hook, &context).await {
            return mutation;
        }

        let Some(orchestrator) = context.orchestrator.clone() else {
            return distri_types::HookMutation::none();
        };

        if request.fire_and_forget {
            context
                .emit(AgentEventType::InlineHookRequested {
                    request: request.clone(),
                })
                .await;
            return distri_types::HookMutation::none();
        }

        let hook_id = Uuid::new_v4().to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        orchestrator.inline_hooks.insert(hook_id.clone(), tx);

        let mut request = request;
        request.hook_id = hook_id.clone();
        request.message = message;
        request.plan = plan;
        request.result = result;

        context
            .emit(AgentEventType::InlineHookRequested {
                request: request.clone(),
            })
            .await;

        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(mutation)) => mutation,
            _ => {
                // Clean up any pending sender on timeout/cancel.
                orchestrator.inline_hooks.remove(&hook_id);
                distri_types::HookMutation::none()
            }
        }
    }
}

#[async_trait]
impl AgentHooks for InlineHook {
    async fn on_plan_start(
        &self,
        message: &mut crate::types::Message,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        let mutation = self
            .request(
                "plan_start",
                Some(message.clone()),
                None,
                None,
                context.clone(),
            )
            .await;
        apply_mutation(message, &context, mutation).await;
        Ok(())
    }

    async fn on_plan_end(
        &self,
        message: &mut crate::types::Message,
        context: Arc<ExecutorContext>,
        plan: &[distri_types::PlanStep],
    ) -> Result<(), AgentError> {
        let mutation = self
            .request(
                "plan_end",
                Some(message.clone()),
                Some(plan.to_vec()),
                None,
                context.clone(),
            )
            .await;
        apply_mutation(message, &context, mutation).await;
        Ok(())
    }

    async fn before_execute(
        &self,
        message: &mut crate::types::Message,
        context: Arc<ExecutorContext>,
    ) -> Result<(), AgentError> {
        let mutation = self
            .request(
                "before_execute",
                Some(message.clone()),
                None,
                None,
                context.clone(),
            )
            .await;
        apply_mutation(message, &context, mutation).await;
        Ok(())
    }

    async fn on_step_end(
        &self,
        context: Arc<ExecutorContext>,
        step: &distri_types::PlanStep,
        result: &distri_types::ExecutionResult,
    ) -> Result<(), AgentError> {
        let _ = self
            .request(
                "step_end",
                None,
                Some(vec![step.clone()]),
                Some(result.clone()),
                context,
            )
            .await;
        Ok(())
    }
}

async fn maybe_enrich_browsr(
    hook: &str,
    context: &ExecutorContext,
) -> Option<distri_types::HookMutation> {
    // Only for browsr agent and for early lifecycle hooks.
    if context.agent_id != "browsr" {
        return None;
    }
    if hook != "plan_start" && hook != "before_execute" {
        return None;
    }

    let client = BrowsrClient::from_config(browsr_default_transport());
    let observe = client
        .observe(None, Some(true), Default::default())
        .await
        .ok()?;
    let mut values = std::collections::HashMap::new();
    values.insert(
        "browser_state".to_string(),
        serde_json::json!(observe.state_text),
    );
    values.insert(
        "browser_url".to_string(),
        serde_json::json!(observe.dom_snapshot.url),
    );
    values.insert(
        "browser_title".to_string(),
        serde_json::json!(observe.dom_snapshot.title),
    );
    Some(distri_types::HookMutation {
        dynamic_values: values,
    })
}
