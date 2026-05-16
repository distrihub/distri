//! Core types for the workflow engine.
//!
//! Two layers:
//!
//! - **Definition** (`WorkflowDefinition`, `WorkflowStep`): the static
//!   template — what this workflow IS. Stored alongside the agent
//!   config; never mutated by execution.
//! - **Run** (`WorkflowRun`, `WorkflowStepRun`): runtime state for one
//!   execution — status, current step pointer, shared context, per-step
//!   result/error/timestamps. Built from a definition via
//!   `WorkflowRun::new(definition)` and then mutated by the engine.
//!
//! Status uses the canonical `distri_types::TaskStatus` everywhere.
//! Workflow-specific concepts that don't have a 1:1 TaskStatus value
//! map as follows:
//!
//! | concept | TaskStatus | extra signal |
//! |---|---|---|
//! | step waiting for input / workflow paused | InputRequired | — |
//! | step skipped (skip_if / entry-point) | Canceled | note appended |
//! | step blocked (missing requirement) | Failed | error explains |
//!
//! Phase 2b will replace `WorkflowStateStore` with the cloud
//! `TaskStore` + a `workflow_step_executions` sidecar so runs flow
//! through the canonical task tree.

use chrono::{DateTime, Utc};
use distri_types::TaskStatus;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// Workflow Definition (template)
// ============================================================================

/// A workflow is a DAG of steps. The definition is the *template* — no
/// runtime state lives here. Use `WorkflowRun::new(definition)` to start
/// an execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub id: String,
    pub steps: Vec<WorkflowStep>,
    /// JSON Schema describing required inputs for this workflow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    /// How workflow state is checkpointed between steps.
    #[serde(default)]
    pub checkpoint: CheckpointStrategy,
    /// Named entry points for multi-entry workflows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entry_points: Vec<EntryPoint>,
}

/// Built-in channel commands a workflow may not shadow. Mirrors
/// `distri-gateway` `ChannelCommand::parse`.
pub const BUILTIN_CHANNEL_COMMANDS: &[&str] = &[
    "/start",
    "/stop",
    "/disconnect",
    "/reset",
    "/new",
    "/newsession",
    "/newthread",
    "/status",
    "/debug",
    "/verbose",
    "/help",
    "/switch",
    "/workspace",
    "/context",
    "/ctx",
];

impl WorkflowDefinition {
    pub fn new(steps: Vec<WorkflowStep>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            steps,
            input_schema: None,
            checkpoint: CheckpointStrategy::default(),
            entry_points: vec![],
        }
    }

    pub fn with_id(mut self, id: &str) -> Self {
        self.id = id.to_string();
        self
    }

    pub fn with_checkpoint(mut self, strategy: CheckpointStrategy) -> Self {
        self.checkpoint = strategy;
        self
    }

    pub fn with_entry_points(mut self, entry_points: Vec<EntryPoint>) -> Self {
        self.entry_points = entry_points;
        self
    }

    /// Get an entry point by ID.
    pub fn entry_point(&self, id: &str) -> Option<&EntryPoint> {
        self.entry_points.iter().find(|ep| ep.id == id)
    }

    /// Find all step IDs reachable from the given step (inclusive) by following
    /// depends_on forward. Used by entry-point logic to mark unreachable steps
    /// as skipped at run start.
    pub fn reachable_from(&self, start_step_id: &str) -> std::collections::HashSet<String> {
        use std::collections::{HashSet, VecDeque};

        let mut reachable = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(start_step_id.to_string());

        while let Some(current) = queue.pop_front() {
            if !reachable.insert(current.clone()) {
                continue;
            }
            for step in &self.steps {
                if step.depends_on.contains(&current) && !reachable.contains(&step.id) {
                    queue.push_back(step.id.clone());
                }
            }
        }

        reachable
    }

    /// Validate the channel-command surface declared by entry-point
    /// triggers. Returns a precise error string on the first problem.
    pub fn validate_channel_surface(&self) -> Result<(), String> {
        use distri_types::channel_commands::ChannelTrigger;
        use std::collections::HashSet;

        let step_ids: HashSet<&str> =
            self.steps.iter().map(|s| s.id.as_str()).collect();
        let mut slash_names: HashSet<String> = HashSet::new();
        let mut callback_ids: HashSet<String> = HashSet::new();
        let mut message_count = 0usize;

        for ep in &self.entry_points {
            if !step_ids.contains(ep.starts_at.as_str()) {
                return Err(format!(
                    "entry point '{}' starts_at unknown step '{}'",
                    ep.id, ep.starts_at
                ));
            }
            let Some(trigger) = &ep.trigger else { continue };
            match trigger {
                ChannelTrigger::Slash { name, aliases, .. } => {
                    for n in std::iter::once(name).chain(aliases.iter()) {
                        let lower = n.to_lowercase();
                        if BUILTIN_CHANNEL_COMMANDS.contains(&lower.as_str()) {
                            return Err(format!(
                                "slash command '{n}' shadows a built-in command"
                            ));
                        }
                        if !slash_names.insert(lower.clone()) {
                            return Err(format!(
                                "entry point '{}': slash command '{}' is already declared",
                                ep.id, n
                            ));
                        }
                    }
                }
                ChannelTrigger::Callback { id, .. } => {
                    // Note: cross-validating a Reply step's ReplyButtonSpec::Callback.callback_data
                    // against declared callback ids is intentionally NOT done in v1 (out of plan scope).
                    if !callback_ids.insert(id.clone()) {
                        return Err(format!(
                            "entry point '{}': callback id '{}' is already declared",
                            ep.id, id
                        ));
                    }
                }
                ChannelTrigger::Message {} => message_count += 1,
            }
        }
        if message_count > 1 {
            return Err(format!(
                "workflow declares {message_count} message catch-all entry \
                 points; at most one is allowed"
            ));
        }
        for step in &self.steps {
            if let StepKind::Reply {
                buttons_from,
                button_template,
                ..
            } = &step.kind
            {
                if button_template.is_some() != buttons_from.is_some() {
                    return Err(format!(
                        "reply step '{}': button_template and buttons_from \
                         must be set together",
                        step.id
                    ));
                }
            }
        }
        Ok(())
    }

    /// Detect circular dependencies in the workflow DAG.
    /// Returns `Err` with the cycle description if found.
    pub fn detect_cycles(&self) -> Result<(), String> {
        use std::collections::{HashMap, HashSet};

        let step_ids: HashSet<&str> = self.steps.iter().map(|s| s.id.as_str()).collect();
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
        for step in &self.steps {
            adj.insert(
                step.id.as_str(),
                step.depends_on.iter().map(|s| s.as_str()).collect(),
            );
        }

        let mut visited = HashSet::new();
        let mut in_stack = HashSet::new();

        fn dfs<'a>(
            node: &'a str,
            adj: &HashMap<&'a str, Vec<&'a str>>,
            visited: &mut HashSet<&'a str>,
            in_stack: &mut HashSet<&'a str>,
            path: &mut Vec<&'a str>,
        ) -> Result<(), String> {
            visited.insert(node);
            in_stack.insert(node);
            path.push(node);

            if let Some(deps) = adj.get(node) {
                for &dep in deps {
                    if !visited.contains(dep) {
                        dfs(dep, adj, visited, in_stack, path)?;
                    } else if in_stack.contains(dep) {
                        let cycle_start = path.iter().position(|&n| n == dep).unwrap();
                        let cycle: Vec<&str> = path[cycle_start..].to_vec();
                        return Err(format!(
                            "Circular dependency detected: {} → {}",
                            cycle.join(" → "),
                            dep
                        ));
                    }
                }
            }

            in_stack.remove(node);
            path.pop();
            Ok(())
        }

        let mut path = Vec::new();
        for step in &self.steps {
            if !visited.contains(step.id.as_str()) {
                dfs(
                    step.id.as_str(),
                    &adj,
                    &mut visited,
                    &mut in_stack,
                    &mut path,
                )?;
            }
        }

        // Also check for references to non-existent steps
        for step in &self.steps {
            for dep in &step.depends_on {
                if !step_ids.contains(dep.as_str()) {
                    return Err(format!(
                        "Step '{}' depends on '{}' which does not exist",
                        step.id, dep
                    ));
                }
            }
        }

        Ok(())
    }
}


// ============================================================================
// Workflow Run (execution state)
// ============================================================================

fn default_empty_object() -> serde_json::Value {
    serde_json::json!({})
}

fn default_now() -> DateTime<Utc> {
    Utc::now()
}

/// One execution of a `WorkflowDefinition`. Owns the definition plus all
/// runtime state — status, shared context, per-step status / result /
/// error / timestamps. The engine mutates a `WorkflowRun`.
///
/// `step_runs` is parallel to `definition.steps` (same length, same
/// order).
///
/// **Wire shape**: `definition` is flattened, so a `WorkflowRun`
/// serializes as one flat JSON object with the definition fields
/// (`id`, `steps`, `input_schema`, …) alongside the runtime fields
/// (`status`, `current_step`, `context`, `step_runs`, …). This
/// keeps the persisted run row identical to the legacy monolithic
/// shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    #[serde(flatten)]
    pub definition: WorkflowDefinition,
    #[serde(default)]
    pub status: WorkflowStatus,
    #[serde(default)]
    pub current_step: usize,
    #[serde(default = "default_empty_object")]
    pub context: serde_json::Value,
    #[serde(default)]
    pub notes: Vec<WorkflowNote>,
    #[serde(default)]
    pub step_runs: Vec<WorkflowStepRun>,
    #[serde(default = "default_now")]
    pub created_at: DateTime<Utc>,
    #[serde(default = "default_now")]
    pub updated_at: DateTime<Utc>,
}

/// Per-step runtime state. Parallel to `WorkflowDefinition.steps[i]`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkflowStepRun {
    pub step_id: String,
    #[serde(default)]
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

impl WorkflowRun {
    /// Build a fresh run from a definition. All step_runs start `Pending`,
    /// context is `{}`, status is `Pending`.
    pub fn new(definition: WorkflowDefinition) -> Self {
        let step_runs = definition
            .steps
            .iter()
            .map(|s| WorkflowStepRun {
                step_id: s.id.clone(),
                ..Default::default()
            })
            .collect();
        Self {
            definition,
            status: WorkflowStatus::Pending,
            current_step: 0,
            context: serde_json::json!({}),
            notes: vec![],
            step_runs,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Convenience for callers that want to skip the explicit
    /// definition struct (mostly tests).
    pub fn from_steps(steps: Vec<WorkflowStep>) -> Self {
        Self::new(WorkflowDefinition::new(steps))
    }

    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = context;
        self
    }

    pub fn with_id(mut self, id: &str) -> Self {
        self.definition.id = id.to_string();
        self
    }

    pub fn with_checkpoint(mut self, strategy: CheckpointStrategy) -> Self {
        self.definition.checkpoint = strategy;
        self
    }

    pub fn with_entry_points(mut self, entry_points: Vec<EntryPoint>) -> Self {
        self.definition.entry_points = entry_points;
        self
    }

    pub fn id(&self) -> &str {
        &self.definition.id
    }

    pub fn steps(&self) -> &[WorkflowStep] {
        &self.definition.steps
    }

    pub fn step(&self, idx: usize) -> &WorkflowStep {
        &self.definition.steps[idx]
    }

    pub fn step_run(&self, idx: usize) -> &WorkflowStepRun {
        &self.step_runs[idx]
    }

    pub fn step_run_mut(&mut self, idx: usize) -> &mut WorkflowStepRun {
        &mut self.step_runs[idx]
    }

    pub fn step_run_by_id(&self, step_id: &str) -> Option<&WorkflowStepRun> {
        self.step_runs.iter().find(|s| s.step_id == step_id)
    }

    pub fn step_run_by_id_mut(&mut self, step_id: &str) -> Option<&mut WorkflowStepRun> {
        self.step_runs.iter_mut().find(|s| s.step_id == step_id)
    }

    /// Apply an entry point: mark steps not reachable from `starts_at` as
    /// Skipped, pre-populate their results from `preset_results`, and
    /// merge those into context so downstream `{steps.X}` references work.
    pub fn apply_entry_point(mut self, entry_point_id: &str) -> Result<Self, String> {
        let ep = self
            .definition
            .entry_points
            .iter()
            .find(|ep| ep.id == entry_point_id)
            .ok_or_else(|| format!("Entry point '{}' not found", entry_point_id))?
            .clone();

        if !self.definition.steps.iter().any(|s| s.id == ep.starts_at) {
            return Err(format!(
                "Entry point '{}' references step '{}' which does not exist",
                entry_point_id, ep.starts_at
            ));
        }

        let reachable = self.definition.reachable_from(&ep.starts_at);

        for (i, step) in self.definition.steps.iter().enumerate() {
            if !reachable.contains(&step.id) {
                self.step_runs[i].status = StepStatus::Skipped;
                if let Some(result) = ep.preset_results.get(&step.id) {
                    self.step_runs[i].result = Some(result.clone());
                }
            }
        }

        if let Some(ctx) = self.context.as_object_mut() {
            let steps = ctx
                .entry("steps")
                .or_insert(serde_json::json!({}))
                .as_object_mut()
                .expect("steps must be an object");
            for (step_id, result) in &ep.preset_results {
                steps.insert(step_id.clone(), result.clone());
            }
        }

        Ok(self)
    }

    /// Initialize the run with validated input. Input is validated
    /// against `definition.input_schema` if present, then merged into
    /// `context`. Status flips to Running.
    pub fn with_input(mut self, input: serde_json::Value) -> Result<Self, String> {
        if let Some(ref schema_value) = self.definition.input_schema {
            let validator = jsonschema::validator_for(schema_value)
                .map_err(|e| format!("Invalid input_schema: {e}"))?;

            if !validator.is_valid(&input) {
                let errors: Vec<String> = validator
                    .iter_errors(&input)
                    .map(|e| format!("{}", e))
                    .collect();
                return Err(format!("Input validation failed: {}", errors.join("; ")));
            }
        }

        if let (Some(ctx), Some(inp)) = (self.context.as_object_mut(), input.as_object()) {
            for (k, v) in inp {
                ctx.insert(k.clone(), v.clone());
            }
            ctx.insert("input".to_string(), input.clone());
        }

        self.status = WorkflowStatus::Running;
        self.updated_at = Utc::now();
        Ok(self)
    }

    /// First pending step, if any.
    pub fn next_pending_step(&self) -> Option<(usize, &WorkflowStep)> {
        self.step_runs
            .iter()
            .enumerate()
            .find(|(_, s)| s.status == StepStatus::Pending)
            .map(|(i, _)| (i, &self.definition.steps[i]))
    }

    /// All steps that can run now: pending + all dependencies completed.
    /// Pure query — does not mutate.
    pub fn runnable_steps(&self) -> Vec<(usize, &WorkflowStep)> {
        let mut runnable = vec![];
        for (i, step) in self.definition.steps.iter().enumerate() {
            if self.step_runs[i].status != StepStatus::Pending {
                continue;
            }
            let deps_met = step.depends_on.iter().all(|dep_id| {
                self.definition
                    .steps
                    .iter()
                    .zip(self.step_runs.iter())
                    .any(|(s, sr)| {
                        &s.id == dep_id
                            && matches!(sr.status, StepStatus::Done | StepStatus::Skipped)
                    })
            });
            if deps_met {
                runnable.push((i, step));
            }
        }
        runnable
    }

    pub fn is_complete(&self) -> bool {
        self.step_runs.iter().all(|s| {
            matches!(
                s.status,
                StepStatus::Done | StepStatus::Skipped | StepStatus::Blocked
            )
        })
    }

    pub fn is_waiting_for_input(&self) -> bool {
        self.step_runs
            .iter()
            .any(|s| s.status == StepStatus::WaitingForInput)
    }

    pub fn waiting_step(&self) -> Option<(usize, &WorkflowStep)> {
        self.step_runs
            .iter()
            .enumerate()
            .find(|(_, s)| s.status == StepStatus::WaitingForInput)
            .map(|(i, _)| (i, &self.definition.steps[i]))
    }

    /// Resume a paused run by providing input for the waiting step.
    pub fn resume_step(
        &mut self,
        step_id: &str,
        result: serde_json::Value,
    ) -> Result<usize, String> {
        let idx = self
            .step_runs
            .iter()
            .position(|s| s.step_id == step_id && s.status == StepStatus::WaitingForInput)
            .ok_or_else(|| {
                format!(
                    "Step '{}' not found or not in waiting_for_input state",
                    step_id
                )
            })?;

        self.step_runs[idx].status = StepStatus::Done;
        self.step_runs[idx].result = Some(result.clone());
        self.step_runs[idx].completed_at = Some(Utc::now());

        if let Some(ctx) = self.context.as_object_mut() {
            let steps = ctx
                .entry("steps")
                .or_insert(serde_json::json!({}))
                .as_object_mut()
                .expect("steps must be an object");
            steps.insert(step_id.to_string(), result);
        }

        self.status = WorkflowStatus::Running;
        self.updated_at = Utc::now();
        Ok(idx)
    }

    /// Stuck: blocked steps, nothing running, no path forward.
    pub fn is_stuck(&self) -> bool {
        let has_blocked = self
            .step_runs
            .iter()
            .any(|s| s.status == StepStatus::Blocked);
        let has_pending = self
            .step_runs
            .iter()
            .any(|s| s.status == StepStatus::Pending);
        let has_running = self
            .step_runs
            .iter()
            .any(|s| s.status == StepStatus::Running);

        if !has_blocked || has_running {
            return false;
        }

        if !has_pending {
            return true;
        }

        !self
            .definition
            .steps
            .iter()
            .zip(self.step_runs.iter())
            .any(|(step, run)| {
                run.status == StepStatus::Pending
                    && step.depends_on.iter().all(|dep_id| {
                        self.definition
                            .steps
                            .iter()
                            .zip(self.step_runs.iter())
                            .any(|(s, sr)| {
                                &s.id == dep_id
                                    && matches!(
                                        sr.status,
                                        StepStatus::Done
                                            | StepStatus::Pending
                                            | StepStatus::Running
                                    )
                            })
                    })
            })
    }

    pub fn has_failed(&self) -> bool {
        self.step_runs
            .iter()
            .any(|s| s.status == StepStatus::Failed)
    }

    /// Append a note to the run's log.
    pub fn add_note(&mut self, step_id: &str, message: &str) {
        self.notes.push(WorkflowNote {
            step_id: step_id.to_string(),
            message: message.to_string(),
            at: Utc::now(),
        });
        self.updated_at = Utc::now();
    }

    /// Validate the underlying DAG. Convenience that delegates to the
    /// definition's `detect_cycles`.
    pub fn detect_cycles(&self) -> Result<(), String> {
        self.definition.detect_cycles()
    }
}

// ============================================================================
// Entry Point — named starting positions for multi-entry workflows
// ============================================================================

/// A named entry point into a workflow.
/// Allows workflows to be started at different steps depending on context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryPoint {
    /// Unique identifier for this entry point (e.g., "import_from_docs", "grade_only").
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Optional description of when to use this entry point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The step ID where execution begins.
    pub starts_at: String,
    /// Pre-populated step results for steps that are skipped.
    /// Maps step_id → result value. These steps are marked Done before execution starts.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub preset_results: HashMap<String, serde_json::Value>,
    /// Required input fields for this entry point (for UI/validation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_inputs: Vec<String>,
    /// How a channel user reaches this entry point (slash command,
    /// callback button, or free-text catch-all). `None` = not channel-
    /// reachable (e.g. an internal or scheduled entry point).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<distri_types::channel_commands::ChannelTrigger>,
}

// ============================================================================
// Workflow Step (template)
// ============================================================================

/// A single step in a workflow. **Template only** — runtime status /
/// result / timestamps live on `WorkflowStepRun`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: String,
    pub label: String,
    pub kind: StepKind,
    /// IDs of steps that must complete before this one can run.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Execution mode for this step.
    #[serde(default)]
    pub execution: StepExecution,
    /// Capabilities required to run this step.
    #[serde(default)]
    pub requires: Vec<StepRequirement>,
    /// Optional explicit input mapping for this step.
    /// Values can reference `{input.X}`, `{steps.step_id.X}`, `{env.X}`.
    /// If omitted, the step receives the full execution context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    /// Skip this step if the expression evaluates to true against the workflow context.
    /// Expression format: `{input.field_name}` — truthy check (field exists and is not null/false/empty).
    /// Supports negation: `!{input.field_name}` — skip if field is absent/falsy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_if: Option<String>,
}

impl WorkflowStep {
    fn new_step(id: &str, label: &str, kind: StepKind) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            kind,
            depends_on: vec![],
            execution: StepExecution::Sequential,
            requires: vec![],
            input: None,
            skip_if: None,
        }
    }

    pub fn api_call(id: &str, label: &str, method: &str, url: &str) -> Self {
        Self::new_step(
            id,
            label,
            StepKind::ApiCall {
                method: method.to_string(),
                url: url.to_string(),
                body: None,
                headers: None,
            },
        )
    }

    pub fn agent_run(id: &str, label: &str, agent_id: &str, prompt: &str) -> Self {
        Self::new_step(
            id,
            label,
            StepKind::AgentRun {
                agent_id: agent_id.to_string(),
                prompt: prompt.to_string(),
                tools: vec![],
                skills: vec![],
                model: None,
                max_iterations: None,
            },
        )
    }

    pub fn script(id: &str, label: &str, command: &str) -> Self {
        Self::new_step(
            id,
            label,
            StepKind::Script {
                command: command.to_string(),
                args: vec![],
                cwd: None,
                env: None,
                timeout_secs: None,
                output_format: None,
                shell: None,
            },
        )
    }

    pub fn tool_call(id: &str, label: &str, tool_name: &str, input: serde_json::Value) -> Self {
        Self::new_step(
            id,
            label,
            StepKind::ToolCall {
                tool_name: tool_name.to_string(),
                input,
                agent_id: None,
            },
        )
    }

    pub fn condition(
        id: &str,
        label: &str,
        expression: &str,
        if_true: StepKind,
        if_false: Option<StepKind>,
    ) -> Self {
        Self::new_step(
            id,
            label,
            StepKind::Condition {
                expression: expression.to_string(),
                if_true: Box::new(if_true),
                if_false: if_false.map(Box::new),
            },
        )
    }

    pub fn checkpoint(id: &str, label: &str, message: &str) -> Self {
        Self::new_step(
            id,
            label,
            StepKind::Checkpoint {
                message: message.to_string(),
            },
        )
    }

    pub fn wait_for_input(id: &str, label: &str, message: &str) -> Self {
        Self::new_step(
            id,
            label,
            StepKind::WaitForInput {
                message: message.to_string(),
                schema: None,
            },
        )
    }

    pub fn with_body(mut self, body: serde_json::Value) -> Self {
        if let StepKind::ApiCall {
            body: ref mut b, ..
        } = self.kind
        {
            *b = Some(body);
        }
        self
    }

    pub fn with_depends_on(mut self, deps: Vec<&str>) -> Self {
        self.depends_on = deps.into_iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn parallel(mut self) -> Self {
        self.execution = StepExecution::Parallel;
        self
    }

    pub fn with_requires(mut self, requires: Vec<StepRequirement>) -> Self {
        self.requires = requires;
        self
    }

    pub fn with_cwd(mut self, cwd: &str) -> Self {
        if let StepKind::Script { cwd: ref mut c, .. } = self.kind {
            *c = Some(cwd.to_string());
        }
        self
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        if let StepKind::Script {
            timeout_secs: ref mut t,
            ..
        } = self.kind
        {
            *t = Some(secs);
        }
        self
    }

    pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
        if let StepKind::Script { env: ref mut e, .. } = self.kind {
            *e = Some(env);
        }
        self
    }

    pub fn with_input_mapping(mut self, input: serde_json::Value) -> Self {
        self.input = Some(input);
        self
    }

    pub fn with_skip_if(mut self, expression: &str) -> Self {
        self.skip_if = Some(expression.to_string());
        self
    }
}

// ============================================================================
// Step Kind — what the step does
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StepKind {
    /// HTTP API call
    ApiCall {
        method: String,
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        body: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        headers: Option<HashMap<String, String>>,
    },

    /// Shell script / command
    Script {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env: Option<HashMap<String, String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_format: Option<ScriptOutputFormat>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell: Option<ShellType>,
    },

    /// Delegate to a Distri agent (sub-agent run)
    AgentRun {
        agent_id: String,
        prompt: String,
        #[serde(default)]
        tools: Vec<String>,
        /// Skills to load for this agent step
        #[serde(default)]
        skills: Vec<String>,
        /// Override model for this step
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// Limit agent loop iterations
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_iterations: Option<u32>,
    },

    /// Single tool invocation — not a full agent loop
    ToolCall {
        /// Tool name (must be registered)
        tool_name: String,
        /// Tool input parameters
        input: serde_json::Value,
        /// Agent context to execute in (for tools needing agent-scoped permissions)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
    },

    /// Conditional branch — evaluates expression against context
    Condition {
        expression: String,
        if_true: Box<StepKind>,
        #[serde(skip_serializing_if = "Option::is_none")]
        if_false: Option<Box<StepKind>>,
    },

    /// No-op / marker step (for documentation or manual checkpoints)
    Checkpoint { message: String },

    /// Pause execution and wait for external/human input before continuing.
    /// The workflow saves state and stops. A resume call provides the input
    /// as the step result and continues from here.
    WaitForInput {
        /// Message to display to the human (what input is needed)
        message: String,
        /// Optional JSON Schema describing the expected input shape
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<serde_json::Value>,
    },

    /// Emit a channel reply (text + optional buttons). Rendered by the
    /// gateway per channel. `text` / button fields support the standard
    /// `{input.x}` / `{steps.id.x}` interpolation; `button_template`
    /// fields additionally support `{item.x}` per `buttons_from`
    /// element.
    Reply {
        text: String,
        /// Static buttons (rows: outer = top-to-bottom).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        buttons: Vec<Vec<distri_types::channel_commands::ReplyButtonSpec>>,
        /// Context path resolving to an array; one button per element.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        buttons_from: Option<String>,
        /// Template applied per `buttons_from` element.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        button_template:
            Option<distri_types::channel_commands::ReplyButtonSpec>,
    },
}

// ============================================================================
// Step Requirement — what a step needs to run
// ============================================================================

/// A capability required to execute a step.
/// Uses namespaced skill identifiers:
/// - `native:shell`, `native:browser`, `native:network` — built-in
/// - `{provider}:{service}` — connections (e.g., `google:drive`, `slack:chat`)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepRequirement {
    /// Namespaced skill identifier.
    pub skill: String,
    /// Required permissions/scopes within the skill.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Optional extra constraints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

impl StepRequirement {
    /// Create a native skill requirement (prefixed with "native:").
    pub fn native(skill: &str) -> Self {
        Self {
            skill: format!("native:{}", skill),
            permissions: vec![],
            config: None,
        }
    }

    /// Create a connection requirement (e.g., "google:drive").
    pub fn connection(provider: &str, service: &str) -> Self {
        Self {
            skill: format!("{}:{}", provider, service),
            permissions: vec![],
            config: None,
        }
    }

    pub fn with_permissions(mut self, perms: Vec<&str>) -> Self {
        self.permissions = perms.into_iter().map(|s| s.to_string()).collect();
        self
    }

    /// Get the namespace (part before ':').
    pub fn namespace(&self) -> Option<&str> {
        self.skill.split(':').next()
    }

    /// Get the skill name (part after ':').
    pub fn skill_name(&self) -> Option<&str> {
        self.skill.split(':').nth(1)
    }

    /// Check if this is a native skill.
    pub fn is_native(&self) -> bool {
        self.skill.starts_with("native:")
    }

    /// Validate the requirement. Returns error message if invalid.
    pub fn validate(&self) -> Result<(), String> {
        if !self.skill.contains(':') {
            return Err(format!(
                "Invalid skill identifier '{}': must be namespaced (e.g., 'native:shell', 'google:drive')",
                self.skill
            ));
        }

        if self.is_native() {
            let known = ["shell", "browser", "network", "agent", "tool"];
            if let Some(name) = self.skill_name() {
                if !known.contains(&name) {
                    return Err(format!(
                        "Unknown native skill '{}'. Known: {:?}",
                        name, known
                    ));
                }
            }
        }

        Ok(())
    }
}

// ============================================================================
// Checkpoint Strategy
// ============================================================================

/// How workflow state is checkpointed between steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CheckpointStrategy {
    /// Redis-based, thread+task scoped, auto-TTL.
    Internal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ttl_secs: Option<u64>,
    },
    /// Client-registered tool handles persistence.
    /// Tool must support actions: save, load, list.
    External { tool_name: String },
}

impl Default for CheckpointStrategy {
    fn default() -> Self {
        CheckpointStrategy::Internal { ttl_secs: None }
    }
}

/// Metadata about a checkpoint snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointMeta {
    pub checkpoint_id: String,
    pub workflow_id: String,
    pub step_id: String,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Enums
// ============================================================================

/// Top-level run status. Engine-internal; external surfaces translate
/// to `distri_types::TaskStatus` via `From`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    #[default]
    Pending,
    Running,
    /// Waiting for human/external input (`WaitForInput` step).
    Paused,
    Completed,
    Failed,
    /// All remaining steps are blocked — requirements cannot be met.
    Blocked,
}

impl From<WorkflowStatus> for TaskStatus {
    fn from(s: WorkflowStatus) -> Self {
        match s {
            WorkflowStatus::Pending => TaskStatus::Pending,
            WorkflowStatus::Running => TaskStatus::Running,
            WorkflowStatus::Paused => TaskStatus::InputRequired,
            WorkflowStatus::Completed => TaskStatus::Completed,
            WorkflowStatus::Failed => TaskStatus::Failed,
            // No 1:1 TaskStatus for Blocked — surface as Failed; the
            // step-level error fields carry the "missing skills:" reason.
            WorkflowStatus::Blocked => TaskStatus::Failed,
        }
    }
}

/// Per-step phase. Engine-internal; richer than `TaskStatus` because
/// the engine cares about the difference between "blocked on a missing
/// requirement" (cannot start) and "failed during execution" (tried,
/// errored). External surfaces translate via `From<StepStatus>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    #[default]
    Pending,
    /// Requirements not met — cannot execute.
    Blocked,
    Running,
    Done,
    Failed,
    Skipped,
    /// Step is waiting for external/human input. Workflow is paused.
    WaitingForInput,
}

impl From<StepStatus> for TaskStatus {
    fn from(s: StepStatus) -> Self {
        match s {
            StepStatus::Pending => TaskStatus::Pending,
            // No TaskStatus::Blocked — surface as Failed (with
            // `step_run.error` carrying the missing-requirement reason).
            StepStatus::Blocked => TaskStatus::Failed,
            StepStatus::Running => TaskStatus::Running,
            StepStatus::Done => TaskStatus::Completed,
            StepStatus::Failed => TaskStatus::Failed,
            // Intentionally not run; semantically a deliberate cancel.
            StepStatus::Skipped => TaskStatus::Canceled,
            StepStatus::WaitingForInput => TaskStatus::InputRequired,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StepExecution {
    /// Must wait for previous step to complete.
    #[default]
    Sequential,
    /// Can run in parallel with other parallel steps at the same level.
    Parallel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptOutputFormat {
    Text,
    Json,
    Stream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellType {
    Bash,
    Sh,
    Zsh,
}

// ============================================================================
// Step Result
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub status: StepStatus,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    /// Updates to merge into workflow context for subsequent steps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_updates: Option<serde_json::Value>,
}

impl StepResult {
    pub fn done(result: serde_json::Value) -> Self {
        Self {
            status: StepStatus::Done,
            result: Some(result),
            error: None,
            context_updates: None,
        }
    }

    pub fn done_with_context(result: serde_json::Value, updates: serde_json::Value) -> Self {
        Self {
            status: StepStatus::Done,
            result: Some(result),
            error: None,
            context_updates: Some(updates),
        }
    }

    pub fn failed(error: &str) -> Self {
        Self {
            status: StepStatus::Failed,
            result: None,
            error: Some(error.to_string()),
            context_updates: None,
        }
    }

    pub fn skipped() -> Self {
        Self {
            status: StepStatus::Skipped,
            result: None,
            error: None,
            context_updates: None,
        }
    }
}

// ============================================================================
// Workflow Note
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNote {
    pub step_id: String,
    pub message: String,
    pub at: DateTime<Utc>,
}

// ============================================================================
// Workflow Run Summary (returned at end of execution)
// ============================================================================

/// Snapshot of one finished step in a `WorkflowRunSummary`.
///
/// Surfaces `distri_types::TaskStatus` at the boundary — engine-internal
/// `StepStatus` distinctions (Blocked / Skipped) translate via
/// `StepStatus → TaskStatus`. The original phase is still recoverable
/// from the `error` field for Blocked steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStepSummary {
    pub id: String,
    pub label: String,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Final summary of a workflow run — id, terminal status, and one row
/// per step. Returned to callers (e.g. the WorkflowAgent invoke result)
/// instead of an ad-hoc JSON shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunSummary {
    pub workflow_id: String,
    pub status: TaskStatus,
    pub steps: Vec<WorkflowStepSummary>,
}

impl WorkflowRunSummary {
    /// Build a summary from a finished `WorkflowRun` and its terminal
    /// `WorkflowStatus`. Translates statuses to `TaskStatus` at the
    /// boundary so consumers don't need to know about the engine's
    /// internal enums.
    pub fn from_run(run: &WorkflowRun, status: WorkflowStatus) -> Self {
        let steps = run
            .steps()
            .iter()
            .zip(run.step_runs.iter())
            .map(|(step, sr)| WorkflowStepSummary {
                id: step.id.clone(),
                label: step.label.clone(),
                status: sr.status.into(),
                result: sr.result.clone(),
                error: sr.error.clone(),
            })
            .collect();
        Self {
            workflow_id: run.id().to_string(),
            status: status.into(),
            steps,
        }
    }
}

// ============================================================================
// Workflow Events (for streaming to clients)
// ============================================================================

/// Events emitted during workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum WorkflowEvent {
    /// Workflow started
    WorkflowStarted {
        workflow_id: String,
        total_steps: usize,
    },
    /// A step started executing
    StepStarted {
        workflow_id: String,
        step_id: String,
        step_label: String,
    },
    /// A step completed successfully
    StepCompleted {
        workflow_id: String,
        step_id: String,
        step_label: String,
        result: Option<serde_json::Value>,
    },
    /// A step failed
    StepFailed {
        workflow_id: String,
        step_id: String,
        step_label: String,
        error: String,
    },
    /// A step is waiting for external/human input
    StepWaiting {
        workflow_id: String,
        step_id: String,
        step_label: String,
        message: String,
        schema: Option<serde_json::Value>,
    },
    /// Workflow completed (all steps done or failed)
    WorkflowCompleted {
        workflow_id: String,
        status: WorkflowStatus,
        steps_done: usize,
        steps_failed: usize,
    },
}
