use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{ExecutionResult, Message, PlanStep};

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HookKind {
    PlanStart,
    PlanEnd,
    BeforeExecute,
    StepEnd,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct HookContext {
    pub agent_id: String,
    pub thread_id: String,
    pub task_id: String,
    pub run_id: String,
}

impl HookContext {
    pub fn from_parts(agent_id: &str, thread_id: &str, task_id: &str, run_id: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            thread_id: thread_id.to_string(),
            task_id: task_id.to_string(),
            run_id: run_id.to_string(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct HookMutation {
    #[serde(default)]
    pub dynamic_values: HashMap<String, serde_json::Value>,
}

impl HookMutation {
    pub fn none() -> Self {
        Self {
            dynamic_values: HashMap::new(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct InlineHookRequest {
    pub hook: String,
    pub hook_id: String,
    pub context: HookContext,
    pub timeout_ms: u64,
    #[serde(default)]
    pub fire_and_forget: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Vec<PlanStep>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ExecutionResult>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct InlineHookResponse {
    pub hook_id: String,
    pub mutation: HookMutation,
}
