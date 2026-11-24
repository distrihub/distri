//! The script SDK's host side (`@distri/runtime`, distrihub/distri-cloud#304, phase 3).
//!
//! A workflow `code` step runs in a sandbox. When the host offers the SDK
//! ([`AgentOrchestratorBuilder::with_script_sdk_url`](crate::AgentOrchestratorBuilder::with_script_sdk_url)),
//! the step mints a **run token** here before it starts the script and revokes it when the
//! script ends. The token is bound to the step's workspace, run and step and to the executor
//! context the step runs in; it is never a user or API key, and it is never stored or logged.
//!
//! The script calls back (`POST {url}/call`, `Authorization: Bearer <run token>`); the host's
//! route hands the call to [`AgentOrchestrator::script_sdk_call`], which runs it **as the run**:
//! inside the run's user and workspace scope, with the run's tools (connection tokens are
//! injected on the host and never reach the script). Each call is kept as a
//! [`ScriptCallRecord`] on the step and emitted as an `AgentEventType::ScriptCall` on the run.
//!
//! The registry is in process: the callback has to reach the replica running the step.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use distri_types::script_sdk::{ScriptCall, ScriptCallError, ScriptCallRecord, RUN_TOKEN_PREFIX};
use serde_json::{json, Value};

use crate::agent::{AgentOrchestrator, ExecutorContext};
use crate::types::AgentEventType;

/// What a run token stands for.
pub struct ScriptSdkBinding {
    pub workspace_id: Option<String>,
    pub user_id: String,
    pub run_task_id: String,
    pub step_id: String,
    /// The workflow running the step (its state document, its memory space).
    pub agent_id: String,
    /// The context the step runs in (its tools, connection env, notifier origin).
    pub context: Arc<ExecutorContext>,
    pub expires_at: Instant,
    calls: Mutex<Vec<ScriptCallRecord>>,
}

impl ScriptSdkBinding {
    pub fn new(context: Arc<ExecutorContext>, step_id: &str, ttl: Duration) -> Self {
        Self {
            workspace_id: context.workspace_id.clone(),
            user_id: context.user_id.clone(),
            run_task_id: context.task_id.clone(),
            step_id: step_id.to_string(),
            agent_id: context.agent_id.clone(),
            context,
            expires_at: Instant::now() + ttl,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// The calls made so far, in order.
    pub fn calls(&self) -> Vec<ScriptCallRecord> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

/// The live run tokens of this process.
#[derive(Default)]
pub struct ScriptSdkRegistry {
    bindings: dashmap::DashMap<String, Arc<ScriptSdkBinding>>,
}

impl ScriptSdkRegistry {
    /// A fresh run token for `binding`.
    pub fn mint(&self, binding: ScriptSdkBinding) -> String {
        self.sweep();
        let token = format!(
            "{RUN_TOKEN_PREFIX}{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        self.bindings.insert(token.clone(), Arc::new(binding));
        token
    }

    /// The binding of a live token (`None` when unknown, revoked or expired).
    pub fn get(&self, token: &str) -> Option<Arc<ScriptSdkBinding>> {
        if !token.starts_with(RUN_TOKEN_PREFIX) {
            return None;
        }
        let b = self.bindings.get(token).map(|b| b.clone())?;
        if Instant::now() >= b.expires_at {
            self.bindings.remove(token);
            return None;
        }
        Some(b)
    }

    /// End a token (the step ended); returns its binding with the calls it recorded.
    pub fn revoke(&self, token: &str) -> Option<Arc<ScriptSdkBinding>> {
        self.bindings.remove(token).map(|(_, b)| b)
    }

    /// How many tokens are live.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    fn sweep(&self) {
        let now = Instant::now();
        self.bindings.retain(|_, b| b.expires_at > now);
    }
}

/// Revokes a run token when the step's script ends (also on cancel or panic).
pub(crate) struct RunTokenGuard {
    registry: Arc<ScriptSdkRegistry>,
    token: String,
}

impl RunTokenGuard {
    pub(crate) fn new(registry: Arc<ScriptSdkRegistry>, token: String) -> Self {
        Self { registry, token }
    }

    /// Revoke now and return the calls the script made.
    pub(crate) fn finish(self) -> Vec<ScriptCallRecord> {
        self.registry
            .revoke(&self.token)
            .map(|b| b.calls())
            .unwrap_or_default()
    }
}

impl Drop for RunTokenGuard {
    fn drop(&mut self) {
        self.registry.revoke(&self.token);
    }
}

fn workspace_uuid(ws: Option<&str>) -> Option<uuid::Uuid> {
    ws.and_then(|w| uuid::Uuid::parse_str(w).ok())
}

impl AgentOrchestrator {
    /// One call from a sandboxed script, authorized by its run token. See the module docs.
    pub async fn script_sdk_call(
        self: &Arc<Self>,
        token: &str,
        call: ScriptCall,
    ) -> Result<Value, ScriptCallError> {
        let binding = self
            .script_sdk
            .get(token)
            .ok_or(ScriptCallError::Unauthorized)?;
        let me = self.clone();
        let b = binding.clone();
        let c = call.clone();
        let result = distri_auth::context::with_user_and_workspace(
            binding.user_id.clone(),
            workspace_uuid(binding.workspace_id.as_deref()),
            async move { me.run_script_call(&b, c).await },
        )
        .await;

        let record = {
            let mut calls = binding
                .calls
                .lock()
                .map_err(|_| ScriptCallError::Failed("script call log poisoned".into()))?;
            let record = ScriptCallRecord {
                seq: calls.len() as u32 + 1,
                op: call.op().to_string(),
                at: Utc::now(),
                ok: result.is_ok(),
                side_effect: call.side_effect(),
                summary: call.summary(),
                error: result.as_ref().err().map(|e| e.to_string()),
            };
            calls.push(record.clone());
            record
        };
        tracing::info!(
            target: "workflow.script_call",
            run_task_id = %binding.run_task_id,
            step_id = %binding.step_id,
            op = %record.op,
            ok = record.ok,
            "script call"
        );
        binding
            .context
            .emit(AgentEventType::ScriptCall {
                step_id: binding.step_id.clone(),
                call: serde_json::to_value(&record).unwrap_or(Value::Null),
            })
            .await;
        result
    }

    async fn run_script_call(
        self: &Arc<Self>,
        b: &ScriptSdkBinding,
        call: ScriptCall,
    ) -> Result<Value, ScriptCallError> {
        let ctx = &b.context;
        let state_key =
            distri_types::WorkflowStateKey::new(b.workspace_id.as_deref(), &b.agent_id);
        match call {
            ScriptCall::Notify { text, buttons } => {
                if text.trim().is_empty() {
                    return Err(ScriptCallError::Invalid("notify needs `text`".into()));
                }
                for b in buttons.iter().flatten() {
                    if let distri_types::channel_commands::ChannelButton::Callback {
                        callback_data,
                        ..
                    } = b
                    {
                        if callback_data.starts_with("wf:op:") {
                            return Err(ScriptCallError::Invalid(
                                "button callbacks may not use the reserved prefix wf:op:".into(),
                            ));
                        }
                    }
                }
                let reply = distri_types::channel_commands::ChannelReply { text, buttons };
                let r = crate::agent::workflow_step_exec::notify(
                    ctx,
                    distri_types::NotifyTarget::Owner,
                    reply,
                )
                .await
                .map_err(ScriptCallError::Failed)?;
                step_result_value(r)
            }
            ScriptCall::StateGet {} => self
                .workflow_state_store
                .get_state(&state_key)
                .await
                .map_err(|e| ScriptCallError::Failed(format!("state: {e}"))),
            ScriptCall::StateSet { patch } => {
                if !patch.is_object() {
                    return Err(ScriptCallError::Invalid(
                        "state.set needs an object".into(),
                    ));
                }
                self.workflow_state_store
                    .merge_state(&state_key, patch)
                    .await
                    .map_err(|e| ScriptCallError::Failed(format!("state: {e}")))
            }
            ScriptCall::Tool { name, input } => {
                let tools = ctx.get_tools().await;
                let Some(tool) = tools.iter().find(|t| t.get_name() == name) else {
                    return Err(ScriptCallError::Failed(format!(
                        "tool '{name}' is not available to this workflow"
                    )));
                };
                let call = distri_types::ToolCall {
                    tool_call_id: uuid::Uuid::new_v4().to_string(),
                    tool_name: name.clone(),
                    input,
                };
                let parts =
                    crate::tools::execute_tool_with_executor_context(tool.as_ref(), call, ctx.clone())
                        .await
                        .map_err(|e| ScriptCallError::Failed(format!("tool {name}: {e}")))?;
                let text = parts
                    .iter()
                    .filter_map(|p| match p {
                        distri_types::Part::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(crate::agent::workflow_step_exec::tool_result_value(text))
            }
            ScriptCall::Agent { agent, prompt } => {
                if agent.trim().is_empty() {
                    return Err(ScriptCallError::Invalid("agent needs a name".into()));
                }
                let r = crate::agent::workflow_step_exec::run_agent_child(
                    ctx,
                    &agent,
                    prompt,
                    None,
                    &b.step_id,
                )
                .await
                .map_err(ScriptCallError::Failed)?;
                step_result_value(r)
            }
            ScriptCall::MemorySignal { signal } => {
                let mut obj = signal.as_object().cloned().unwrap_or_default();
                obj.insert("type".into(), json!("memory"));
                obj.insert("op".into(), json!("signal"));
                let kind: distri_workflow::StepKind = serde_json::from_value(Value::Object(obj))
                    .map_err(|e| ScriptCallError::Invalid(format!("memory.signal: {e}")))?;
                let step = distri_workflow::WorkflowStep::code(&b.step_id, "");
                let step = distri_workflow::WorkflowStep { kind, ..step };
                let r = crate::agent::agent_memory::step::execute_memory_step(
                    &step,
                    &json!({}),
                    ctx,
                )
                .await
                .map_err(ScriptCallError::Failed)?;
                step_result_value(r)
            }
        }
    }
}

fn step_result_value(r: distri_workflow::StepResult) -> Result<Value, ScriptCallError> {
    if r.status == distri_types::TaskStatus::Failed {
        return Err(ScriptCallError::Failed(
            r.error.unwrap_or_else(|| "failed".to_string()),
        ));
    }
    Ok(r.result.unwrap_or(Value::Null))
}
