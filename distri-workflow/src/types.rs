//! Core types for the workflow engine.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// Workflow Definition
// ============================================================================

/// A workflow is a DAG of steps with shared context and tracked state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub id: String,
    pub workflow_type: String,
    pub status: WorkflowStatus,
    pub current_step: usize,
    /// Shared data between steps — each step can read and write to this.
    pub context: serde_json::Value,
    pub steps: Vec<WorkflowStep>,
    pub notes: Vec<WorkflowNote>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkflowDefinition {
    pub fn new(workflow_type: &str, steps: Vec<WorkflowStep>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            workflow_type: workflow_type.to_string(),
            status: WorkflowStatus::Pending,
            current_step: 0,
            context: serde_json::json!({}),
            steps,
            notes: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = context;
        self
    }

    pub fn with_id(mut self, id: &str) -> Self {
        self.id = id.to_string();
        self
    }

    /// Get the next pending step, if any.
    pub fn next_pending_step(&self) -> Option<(usize, &WorkflowStep)> {
        self.steps.iter().enumerate()
            .find(|(_, s)| s.status == StepStatus::Pending)
    }

    /// Get all steps that can run now (parallel-ready pending steps).
    pub fn runnable_steps(&self) -> Vec<(usize, &WorkflowStep)> {
        let mut runnable = vec![];
        for (i, step) in self.steps.iter().enumerate() {
            if step.status != StepStatus::Pending { continue; }

            // Check if all dependencies are done
            let deps_met = step.depends_on.iter().all(|dep_id| {
                self.steps.iter().any(|s| &s.id == dep_id && s.status == StepStatus::Done)
            });

            if deps_met {
                runnable.push((i, step));
            }
        }
        runnable
    }

    /// Check if the workflow is complete (all steps done or skipped).
    pub fn is_complete(&self) -> bool {
        self.steps.iter().all(|s| matches!(s.status, StepStatus::Done | StepStatus::Skipped))
    }

    /// Check if any step has failed.
    pub fn has_failed(&self) -> bool {
        self.steps.iter().any(|s| s.status == StepStatus::Failed)
    }

    /// Add a note to the workflow log.
    pub fn add_note(&mut self, step_id: &str, message: &str) {
        self.notes.push(WorkflowNote {
            step_id: step_id.to_string(),
            message: message.to_string(),
            at: Utc::now(),
        });
        self.updated_at = Utc::now();
    }
}

// ============================================================================
// Workflow Step
// ============================================================================

/// A single step in a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: String,
    pub label: String,
    pub kind: StepKind,
    pub status: StepStatus,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    /// IDs of steps that must complete before this one can run.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Execution mode for this step.
    #[serde(default)]
    pub execution: StepExecution,
}

impl WorkflowStep {
    pub fn api_call(id: &str, label: &str, method: &str, url: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            kind: StepKind::ApiCall {
                method: method.to_string(),
                url: url.to_string(),
                body: None,
                headers: None,
            },
            status: StepStatus::Pending,
            result: None,
            error: None,
            started_at: None,
            completed_at: None,
            depends_on: vec![],
            execution: StepExecution::Sequential,
        }
    }

    pub fn agent_run(id: &str, label: &str, agent_id: &str, prompt: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            kind: StepKind::AgentRun {
                agent_id: agent_id.to_string(),
                prompt: prompt.to_string(),
                tools: vec![],
            },
            status: StepStatus::Pending,
            result: None,
            error: None,
            started_at: None,
            completed_at: None,
            depends_on: vec![],
            execution: StepExecution::Sequential,
        }
    }

    pub fn condition(id: &str, label: &str, expression: &str, if_true: StepKind, if_false: Option<StepKind>) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            kind: StepKind::Condition {
                expression: expression.to_string(),
                if_true: Box::new(if_true),
                if_false: if_false.map(Box::new),
            },
            status: StepStatus::Pending,
            result: None,
            error: None,
            started_at: None,
            completed_at: None,
            depends_on: vec![],
            execution: StepExecution::Sequential,
        }
    }

    pub fn with_body(mut self, body: serde_json::Value) -> Self {
        if let StepKind::ApiCall { body: ref mut b, .. } = self.kind {
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
    },

    /// Delegate to a Distri agent (sub-agent run)
    AgentRun {
        agent_id: String,
        prompt: String,
        #[serde(default)]
        tools: Vec<String>,
    },

    /// Conditional branch — evaluates expression against context
    Condition {
        expression: String,
        if_true: Box<StepKind>,
        #[serde(skip_serializing_if = "Option::is_none")]
        if_false: Option<Box<StepKind>>,
    },

    /// No-op / marker step (for documentation or manual checkpoints)
    Checkpoint {
        message: String,
    },
}

// ============================================================================
// Enums
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
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
        Self { status: StepStatus::Done, result: Some(result), error: None, context_updates: None }
    }

    pub fn done_with_context(result: serde_json::Value, updates: serde_json::Value) -> Self {
        Self { status: StepStatus::Done, result: Some(result), error: None, context_updates: Some(updates) }
    }

    pub fn failed(error: &str) -> Self {
        Self { status: StepStatus::Failed, result: None, error: Some(error.to_string()), context_updates: None }
    }

    pub fn skipped() -> Self {
        Self { status: StepStatus::Skipped, result: None, error: None, context_updates: None }
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
