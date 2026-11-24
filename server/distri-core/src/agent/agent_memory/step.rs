//! The workflow `memory` step: record a signal, write an item, or forget one, with no LLM.

use std::sync::Arc;

use distri_types::configuration::AgentConfig;
use distri_types::memory::{
    MemoryAuthor, MemoryConfig, MemoryOp, MemoryPlan, MemorySignal, MemoryStatus, MemoryUri,
    SignalKind, NO_SCOPE, RULES_FOLDER,
};
use distri_workflow::{
    resolve::{resolve_template, resolve_value},
    MemoryStepOp, StepKind, StepResult, WorkflowStep,
};

use super::{resolve_scope, sanitize_segment, workspace_key};
use crate::agent::ExecutorContext;

/// The `[memory]` of the workflow (or agent) running this context.
async fn own_memory_config(ctx: &ExecutorContext) -> Option<MemoryConfig> {
    let orch = ctx.orchestrator.as_ref()?;
    match orch.get_agent(&ctx.agent_id).await? {
        AgentConfig::WorkflowAgent(def) => def.memory,
        AgentConfig::StandardAgent(def) => def.memory,
    }
    .filter(|m| m.enabled)
}

/// `space` and `scope` of a step are literals or `{state.*}`: never chosen by the run's input
/// (or a previous step's output), so a caller cannot point the step at another account.
fn check_target_template(name: &str, v: &Option<String>) -> Result<(), String> {
    let Some(t) = v else {
        return Ok(());
    };
    let mut rest = t.as_str();
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let close = after.find('}').unwrap_or(after.len());
        let reference = after[..close].trim();
        if !reference.starts_with("state.") {
            return Err(format!(
                "memory step: `{name}` may not come from the run's input or other steps                  (`{{{reference}}}`); use a literal, {{state.*}} or the workflow's [memory].{name}"
            ));
        }
        rest = &after[close.min(after.len())..];
    }
    Ok(())
}

fn opt(v: &Option<String>, wf: &serde_json::Value) -> Option<String> {
    v.as_ref()
        .map(|s| resolve_template(s, wf))
        .filter(|s| !s.trim().is_empty())
}

pub(crate) async fn execute_memory_step(
    step: &WorkflowStep,
    wf: &serde_json::Value,
    ctx: &Arc<ExecutorContext>,
) -> Result<StepResult, String> {
    let StepKind::Memory {
        op,
        space,
        scope,
        kind,
        subject,
        before,
        after,
        context,
        uri,
        l0,
        l1,
        body,
        evidence,
        reason,
    } = &step.kind
    else {
        return Ok(StepResult::failed("not a memory step"));
    };
    let Some(orch) = ctx.orchestrator.as_ref() else {
        return Ok(StepResult::failed("memory step: no orchestrator"));
    };
    // Templates never see the run's secrets.
    let wf = &crate::agent::workflow_agent::context_to_keep(wf);
    for (name, v) in [("space", space), ("scope", scope)] {
        if let Err(e) = check_target_template(name, v) {
            return Ok(StepResult::failed(&e));
        }
    }
    let config = own_memory_config(ctx).await;
    let space = opt(space, wf)
        .map(|s| sanitize_segment(&s))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            config
                .as_ref()
                .map(|c| c.space_for(&ctx.agent_id))
                .unwrap_or_else(|| ctx.agent_id.clone())
        });
    let scope = match opt(scope, wf) {
        Some(s) => {
            let s = sanitize_segment(&s);
            if s.is_empty() {
                NO_SCOPE.to_string()
            } else {
                s
            }
        }
        None => resolve_scope(
            config.as_ref().and_then(|c| c.scope.as_deref()),
            wf.get("input").unwrap_or(&serde_json::Value::Null),
            Some(ctx.user_id.as_str()),
        ),
    };
    let ws = workspace_key(ctx);
    let store = orch.agent_memory_store.clone();
    let author = MemoryAuthor::Agent {
        agent: Some(ctx.agent_id.clone()),
        task_id: Some(ctx.task_id.clone()).filter(|t| !t.is_empty()),
    };

    match op {
        MemoryStepOp::Signal => {
            let Some(kind) = opt(kind, wf) else {
                return Ok(StepResult::failed(
                    "memory signal: `kind` is required (approved, edited, skipped, outcome, note)",
                ));
            };
            let kind: SignalKind = match kind.trim().parse() {
                Ok(k) => k,
                Err(e) => return Ok(StepResult::failed(&format!("memory signal: {e}"))),
            };
            let signal = MemorySignal {
                id: uuid::Uuid::new_v4().to_string(),
                workspace_id: ws,
                space: space.clone(),
                scope: scope.clone(),
                agent: Some(ctx.agent_id.clone()),
                kind,
                subject: opt(subject, wf),
                before: opt(before, wf),
                after: opt(after, wf),
                context: context
                    .as_ref()
                    .map(|c| resolve_value(c, wf))
                    .unwrap_or_else(|| serde_json::json!({})),
                run_task_id: Some(ctx.task_id.clone()).filter(|t| !t.is_empty()),
                reflected_at: None,
                created_at: chrono::Utc::now().timestamp_millis(),
            };
            let id = signal.id.clone();
            match store.bulk_insert_signals(vec![signal]).await {
                Ok(_) => Ok(StepResult::done(serde_json::json!({
                    "signal": format!("sig:{id}"),
                    "space": space,
                    "scope": scope,
                    "kind": kind.as_str(),
                }))),
                Err(e) => Ok(StepResult::failed(&format!("memory signal: {e}"))),
            }
        }
        MemoryStepOp::Write | MemoryStepOp::Forget => {
            let Some(raw) = opt(uri, wf) else {
                return Ok(StepResult::failed("memory step: `uri` is required"));
            };
            let uri = match MemoryUri::parse_relative(&raw, &space, &scope) {
                Ok(u) if u.is_item() => u,
                Ok(u) => {
                    return Ok(StepResult::failed(&format!(
                        "invalid_uri: {u} names a folder; add a slug"
                    )))
                }
                Err(e) => return Ok(StepResult::failed(&format!("invalid_uri: {e}"))),
            };
            if uri.space != space || uri.scope != scope {
                return Ok(StepResult::failed(&format!(
                    "out_of_scope: {uri} is outside mem://{space}/{scope}"
                )));
            }
            if uri.folder == RULES_FOLDER {
                return Ok(StepResult::failed(&format!(
                    "forbidden: {uri}: rules/ is written by the owner only"
                )));
            }
            let folders = config.clone().unwrap_or_default().folders();
            if !folders.contains(&uri.folder) {
                return Ok(StepResult::failed(&format!(
                    "unknown folder `{}` (folders: {})",
                    uri.folder,
                    folders.join(", ")
                )));
            }
            let memory_op = if *op == MemoryStepOp::Forget {
                MemoryOp::Archive {
                    uri: uri.clone(),
                    replacement: None,
                    reason: opt(reason, wf).unwrap_or_else(|| format!("step {}", step.id)),
                }
            } else {
                let Some(l0) = opt(l0, wf) else {
                    return Ok(StepResult::failed("memory write: `l0` is required"));
                };
                let body = opt(body, wf).unwrap_or_else(|| l0.clone());
                let exists = match store.get(&ws, &uri).await {
                    Ok(i) => i
                        .map(|i| i.status != MemoryStatus::Archived)
                        .unwrap_or(false),
                    Err(e) => return Ok(StepResult::failed(&format!("memory write: {e}"))),
                };
                let evidence: Vec<String> =
                    evidence.iter().map(|e| resolve_template(e, wf)).collect();
                if exists {
                    MemoryOp::Replace {
                        uri: uri.clone(),
                        l0,
                        l1: opt(l1, wf),
                        body,
                        evidence,
                        pinned: None,
                    }
                } else {
                    MemoryOp::Create {
                        uri: uri.clone(),
                        l0,
                        l1: opt(l1, wf),
                        body,
                        evidence,
                        pinned: None,
                    }
                }
            };
            let name = memory_op.name();
            match store
                .apply(
                    &ws,
                    &MemoryPlan {
                        ops: vec![memory_op],
                        skipped: vec![],
                    },
                    &author,
                )
                .await
            {
                Ok(changes) => Ok(StepResult::done(serde_json::json!({
                    "uri": uri.to_string(),
                    "op": name,
                    "change_id": changes.first().map(|c| c.id.clone()),
                }))),
                Err(e) => Ok(StepResult::failed(&format!("memory {name}: {e}"))),
            }
        }
    }
}
