//! Reflection: the space's owning agent reads the waiting signals and the current memory, and
//! answers with one JSON [`MemoryPlan`]. The runtime validates the plan (spec §2.5), gives an
//! invalid one back to the model once with the errors, and applies a valid one in a single
//! `apply()` (one transaction, one change-log row per operation). The batch's signals are then
//! marked reflected, whether or not the plan was used.
//!
//! The instruction text is our own (OpenViking is AGPL: ideas only, no text).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use distri_types::configuration::AgentConfig;
use distri_types::memory::{
    plan::{validate_plan, PlanContext},
    MemoryAuthor, MemoryChange, MemoryConfig, MemoryItem, MemoryPlan, MemorySignal, SignalKind,
    SkippedSignal,
};
use distri_types::{ModelSettings, Part, ReflectMode, StandardDefinition, ToolsConfig};
use serde::Serialize;

use super::{session_of, MemorySession};
use crate::agent::{AgentOrchestrator, ExecutorContext};
use crate::AgentError;

/// The error code of a reflection asked for while one is running on the same scope.
pub const REFLECTION_RUNNING: &str = "reflection_running";
/// How many waiting signals one reflection reads.
pub const SIGNALS_PER_REFLECTION: usize = 50;

pub struct ReflectRequest {
    pub workspace_id: String,
    pub space: String,
    pub scope: String,
    pub user_id: Option<String>,
    /// The workspace's model, for an owning agent without its own `model_settings`.
    pub default_model_settings: Option<ModelSettings>,
}

#[derive(Debug, Default, Serialize)]
pub struct ReflectOutcome {
    pub applied: Vec<MemoryChange>,
    pub skipped: Vec<SkippedSignal>,
    /// Signals read (and marked reflected).
    pub signals: usize,
    /// Why the plan was dropped, when it was.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

const INSTRUCTIONS: &str = r#"You keep the long-term memory of another agent. You do not talk to anyone: you read feedback on that agent's recent work and decide what is worth remembering for its future runs.

How to decide:
1. The agent's rules (listed in the task) always win. Never keep anything that contradicts a rule or would lead the agent to break one; list such a signal under "skipped" with the reason.
2. Do not invent. Every item must follow from the signals or from memory that already exists: no made-up facts, customers, numbers or experiences.
3. One signal is one event. On its own it can become at most a `cases` item. Write or change a `style`, `preferences` or `experiences` item only when two or more signals show the same thing, and cite every one of them in "evidence" as "sig:<id>".
4. Prefer changing what exists over adding a near-copy: "patch" an item (its "search" must be an exact piece of the item's current body that occurs once) or "replace" it.
5. Nothing is deleted. "archive" an item the signals contradict, with a "reason" that says where its facts went, and the "replacement" URI when there is one.
6. `rules/` and pinned items belong to the owner: never touch them.
7. "l0" is one line of at most 200 characters, written so it can stand alone in a list. "l1" is a short overview. "body" holds the detail.
8. URIs are mem://SPACE/SCOPE/FOLDER/slug with the space and scope given in the task; slugs are short, lowercase, with dashes.

Answer by calling the `final` tool with exactly one JSON object and nothing around it:
{"ops":[
  {"op":"create","uri":"mem://…","l0":"…","l1":"…","body":"…","evidence":["sig:…","sig:…"]},
  {"op":"patch","uri":"mem://…","search":"…","replace":"…","evidence":["sig:…"]},
  {"op":"replace","uri":"mem://…","l0":"…","body":"…","evidence":["sig:…"]},
  {"op":"archive","uri":"mem://…","replacement":"mem://…","reason":"…"}
 ],
 "skipped":[{"signal":"sig:…","reason":"…"}]}
Every signal of the task appears either as evidence of an operation or under "skipped". An empty "ops" list is a fine answer."#;

fn running() -> &'static Mutex<HashSet<String>> {
    static RUNNING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    RUNNING.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Releases the per-scope reflection slot when dropped.
struct Slot(String);
impl Drop for Slot {
    fn drop(&mut self) {
        running().lock().unwrap().remove(&self.0);
    }
}

/// The LLM agent that reflects for `space`: the agent named like the space, or else the first
/// (by name) declaring it. A workflow cannot reflect (it has no model).
pub async fn owning_agent(
    orch: &AgentOrchestrator,
    space: &str,
) -> Option<(StandardDefinition, MemoryConfig)> {
    let (agents, _) = orch.list_agents(None, None).await;
    let mut candidates: Vec<(StandardDefinition, MemoryConfig)> = agents
        .into_iter()
        .filter_map(|cfg| match cfg {
            AgentConfig::StandardAgent(def) => {
                let mem = def.memory.clone().filter(|m| m.enabled)?;
                (mem.space_for(&def.name) == space).then_some((def, mem))
            }
            AgentConfig::WorkflowAgent(_) => None,
        })
        .collect();
    candidates.sort_by(|a, b| {
        (a.0.name != space)
            .cmp(&(b.0.name != space))
            .then(a.0.name.cmp(&b.0.name))
    });
    candidates.into_iter().next()
}

/// Reflect when `config` is batched and due: `reflect_every` signals waiting, or the oldest
/// waiting for `reflect_after`. A reflection already running on the scope is not an error.
pub async fn reflect_if_due(
    orch: &Arc<AgentOrchestrator>,
    req: ReflectRequest,
    config: &MemoryConfig,
) -> Result<Option<ReflectOutcome>, AgentError> {
    if config.reflect != ReflectMode::Batched {
        return Ok(None);
    }
    let waiting = orch
        .agent_memory_store
        .list_signals(&req.workspace_id, &req.space, Some(&req.scope), true, 1000)
        .await
        .map_err(|e| AgentError::Storage(format!("memory: {e}")))?;
    let Some(oldest) = waiting.iter().map(|s| s.created_at).min() else {
        return Ok(None);
    };
    let now = chrono::Utc::now().timestamp_millis();
    let due = waiting.len() >= config.reflect_every()
        || now - oldest >= config.reflect_after_secs() * 1000;
    if !due {
        return Ok(None);
    }
    match reflect(orch, req).await {
        Ok(out) => Ok(Some(out)),
        Err(e) if e.to_string().contains(REFLECTION_RUNNING) => Ok(None),
        Err(e) => Err(e),
    }
}

fn task_text(
    agent: &str,
    space: &str,
    scope: &str,
    config: &MemoryConfig,
    items: &[MemoryItem],
    signals: &[MemorySignal],
) -> String {
    let mut t = format!(
        "Agent: {agent}\nSpace: {space}\nScope: {scope}\nFolders you may write: {}\n\n",
        config
            .folders()
            .into_iter()
            .filter(|f| f != distri_types::memory::RULES_FOLDER)
            .collect::<Vec<_>>()
            .join(", ")
    );
    t.push_str("## The agent's rules (always win)\n");
    if config.rules.is_empty() {
        t.push_str("(none)\n");
    }
    for r in &config.rules {
        t.push_str(&format!("- {r}\n"));
    }
    t.push_str("\n## Current memory\n");
    if items.is_empty() {
        t.push_str("(empty)\n");
    }
    for i in items {
        t.push_str(&format!(
            "- {}{}: {}\n",
            i.uri,
            if i.pinned { " (pinned)" } else { "" },
            i.l0
        ));
        let body = i.body.trim();
        if !body.is_empty() {
            let short: String = body.chars().take(600).collect();
            t.push_str(&format!("  body: {short}\n"));
        }
    }
    t.push_str("\n## Signals (newest last)\n");
    for s in signals {
        let line = serde_json::json!({
            "id": format!("sig:{}", s.id),
            "kind": s.kind,
            "subject": s.subject,
            "before": s.before,
            "after": s.after,
            "context": s.context,
        });
        t.push_str(&format!("{line}\n"));
    }
    t
}

/// The plan in a model's answer: the JSON object in it (code fences and prose around it are
/// tolerated).
pub fn parse_plan(text: &str) -> Result<MemoryPlan, String> {
    let start = text.find('{').ok_or("the answer has no JSON object")?;
    let end = text.rfind('}').ok_or("the answer has no JSON object")?;
    if end < start {
        return Err("the answer has no JSON object".into());
    }
    serde_json::from_str::<MemoryPlan>(&text[start..=end])
        .map_err(|e| format!("the plan is not valid JSON for a memory plan: {e}"))
}

async fn run_model(
    orch: &Arc<AgentOrchestrator>,
    owner: &StandardDefinition,
    req: &ReflectRequest,
    task_id: &str,
    task: &str,
) -> Result<String, AgentError> {
    let mut def = owner.clone();
    def.instructions = INSTRUCTIONS.to_string();
    def.description = format!("memory reflection for {}", owner.name);
    def.memory = None;
    def.tools = Some(ToolsConfig {
        builtin: vec!["final".to_string()],
        ..Default::default()
    });
    def.sub_agents = vec![];
    def.available_skills = vec![];
    def.connections = vec![];
    def.max_iterations = Some(3);
    let mut config = AgentConfig::StandardAgent(def);
    AgentOrchestrator::apply_agent_overrides(&mut config, None, &req.default_model_settings);
    orch.hydrate_agent_model_settings(&mut config).await?;

    let mut ctx = ExecutorContext::default();
    ctx.orchestrator = Some(orch.clone());
    ctx.agent_id = owner.name.clone();
    ctx.workspace_id = Some(req.workspace_id.clone()).filter(|w| w != super::LOCAL_WORKSPACE);
    ctx.user_id = req.user_id.clone().unwrap_or_default();
    ctx.default_model_settings = req.default_model_settings.clone();
    ctx.task_id = task_id.to_string();
    ctx.thread_id = uuid::Uuid::new_v4().to_string();
    ctx.run_id = uuid::Uuid::new_v4().to_string();
    ctx.verbose = false;
    orch.ensure_thread_exists(
        &owner.name,
        Some(ctx.thread_id.clone()),
        Some("memory reflection"),
        None,
        None,
    )
    .await?;
    let _ = orch
        .stores
        .task_store
        .get_or_create_task(&ctx.thread_id, &ctx.task_id)
        .await;
    let ctx = Arc::new(ctx);
    let out = orch.run_inline_agent(config, task, ctx.clone()).await?;
    let content = match ctx.get_final_result().await {
        Some(serde_json::Value::String(s)) => s,
        Some(v) if v.is_object() && v.get("ops").is_some() => v.to_string(),
        Some(v) => v
            .get("input")
            .map(|i| match i {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_else(|| out.content.clone().unwrap_or_default()),
        None => out.content.unwrap_or_default(),
    };
    Ok(content)
}

/// Run one reflection over a scope's waiting signals now.
pub async fn reflect(
    orch: &Arc<AgentOrchestrator>,
    req: ReflectRequest,
) -> Result<ReflectOutcome, AgentError> {
    let key = format!("{}|{}|{}", req.workspace_id, req.space, req.scope);
    if !running().lock().unwrap().insert(key.clone()) {
        return Err(AgentError::Validation(format!(
            "{REFLECTION_RUNNING}: a reflection of {}/{} is running",
            req.space, req.scope
        )));
    }
    let _slot = Slot(key);
    let store = orch.agent_memory_store.clone();
    let mem_err = |e: anyhow::Error| AgentError::Storage(format!("memory: {e}"));

    let signals = store
        .list_signals(
            &req.workspace_id,
            &req.space,
            Some(&req.scope),
            true,
            SIGNALS_PER_REFLECTION,
        )
        .await
        .map_err(mem_err)?;
    if signals.is_empty() {
        return Ok(ReflectOutcome::default());
    }
    let (owner, config) = owning_agent(orch, &req.space).await.ok_or_else(|| {
        AgentError::InvalidConfiguration(format!(
            "no LLM agent declares memory space `{}`, so nothing can reflect on it",
            req.space
        ))
    })?;
    let (_, items) = store
        .index(&req.workspace_id, &req.space, &req.scope)
        .await
        .map_err(mem_err)?;
    let existing: HashMap<String, MemoryItem> = items
        .iter()
        .map(|i| (i.uri.to_string(), i.clone()))
        .collect();
    let known: HashSet<String> = signals.iter().map(|s| format!("sig:{}", s.id)).collect();
    let folders = config.folders();
    let pctx = PlanContext {
        space: &req.space,
        scope: &req.scope,
        folders: &folders,
        existing: &existing,
        known_signals: Some(&known),
    };

    let task_id = uuid::Uuid::new_v4().to_string();
    let base = task_text(
        &owner.name,
        &req.space,
        &req.scope,
        &config,
        &items,
        &signals,
    );
    let mut task = base.clone();
    let mut plan = None;
    let mut errors = Vec::new();
    for attempt in 0..2 {
        let answer = run_model(orch, &owner, &req, &task_id, &task).await?;
        errors = match parse_plan(&answer) {
            Ok(p) => {
                let errs = validate_plan(&p, &pctx);
                if errs.is_empty() {
                    plan = Some(p);
                    break;
                }
                errs
            }
            Err(e) => vec![e],
        };
        tracing::info!(space = %req.space, scope = %req.scope, attempt, ?errors, "memory: reflection plan refused");
        task = format!(
            "{base}\n## Your previous plan was refused\n{}\nAnswer again with a corrected plan.\n",
            errors
                .iter()
                .map(|e| format!("- {e}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    let now = chrono::Utc::now().timestamp_millis();
    let ids: Vec<String> = signals.iter().map(|s| s.id.clone()).collect();
    let mut outcome = ReflectOutcome {
        signals: signals.len(),
        task_id: Some(task_id.clone()),
        agent: Some(owner.name.clone()),
        ..Default::default()
    };
    match plan {
        Some(plan) => {
            if !plan.ops.is_empty() {
                outcome.applied = store
                    .apply(
                        &req.workspace_id,
                        &plan,
                        &MemoryAuthor::Reflection {
                            agent: Some(owner.name.clone()),
                            task_id: Some(task_id),
                        },
                    )
                    .await
                    .map_err(mem_err)?;
            }
            outcome.skipped = plan.skipped;
        }
        None => {
            tracing::warn!(space = %req.space, scope = %req.scope, ?errors, "memory: reflection plan dropped after one retry");
            outcome.errors = errors;
        }
    }
    store
        .mark_reflected(&req.workspace_id, &ids, now)
        .await
        .map_err(mem_err)?;
    Ok(outcome)
}

/// Compaction of a run with memory queues its summary as a `note` signal, so the next
/// reflection sees what was summarised away.
pub async fn queue_on_compaction(
    ctx: &ExecutorContext,
    tier: &distri_types::events::CompactionTier,
    summary: Option<&str>,
) {
    let Some(session) = session_of(ctx).await else {
        return;
    };
    if session.config.reflect == ReflectMode::Off || session.config.read_only {
        return;
    }
    let Some(orch) = ctx.orchestrator.as_ref() else {
        return;
    };
    let signal = compaction_signal(&session, ctx, tier, summary);
    if let Err(e) = orch
        .agent_memory_store
        .bulk_insert_signals(vec![signal])
        .await
    {
        tracing::warn!(error = %e, "memory: could not queue the compaction for reflection");
    }
}

fn compaction_signal(
    session: &MemorySession,
    ctx: &ExecutorContext,
    tier: &distri_types::events::CompactionTier,
    summary: Option<&str>,
) -> MemorySignal {
    MemorySignal {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: session.workspace_id.clone(),
        space: session.space.clone(),
        scope: session.scope(),
        agent: Some(session.agent.clone()),
        kind: SignalKind::Note,
        subject: Some(format!("compaction:{}", ctx.task_id)),
        before: None,
        after: Some(
            summary
                .map(str::to_string)
                .unwrap_or_else(|| format!("the run's context was compacted ({tier:?})")),
        ),
        context: serde_json::json!({ "tier": format!("{tier:?}").to_lowercase() }),
        run_task_id: Some(ctx.task_id.clone()).filter(|t| !t.is_empty()),
        reflected_at: None,
        created_at: chrono::Utc::now().timestamp_millis(),
    }
}

/// The text parts of a tool result, joined (used by tests reading tool output).
pub fn parts_text(parts: &[Part]) -> String {
    parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => t.clone(),
            Part::Data(v) => v.to_string(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
