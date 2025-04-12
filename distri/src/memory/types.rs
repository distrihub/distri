use serde::{Deserialize, Serialize};
use std::any::Any;

use crate::types::{Message, MessageContent, MessageRole, ToolCall};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentError {
    pub message: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MemoryStep {
    // What needs to be done defined as a task
    Task(TaskStep),
    // Comes up with a plan how to execute that task
    Planning(PlanningStep),
    // Entire object represented what happened executing the plan  involving tools
    Action(ActionStep),
    // System prompt for the agent
    System(SystemStep),
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SystemStep {
    pub system_prompt: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ActionStep {
    pub model_input_messages: Option<Vec<Message>>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    pub step_number: Option<i32>,
    pub error: Option<AgentError>,
    pub duration: Option<f64>,
    pub model_output_message: Option<Message>,
    pub model_output: Option<String>,
    pub observations: Option<String>,
    pub observations_images: Option<Vec<String>>,
    pub action_output: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlanningStep {
    pub model_input_messages: Vec<Message>,
    pub model_output_message_facts: Message,
    pub facts: String,
    pub model_output_message_plan: Message,
    pub plan: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TaskStep {
    pub task: String,
    pub task_images: Option<Vec<String>>,
}

pub trait AgentMemory: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn new() -> Self
    where
        Self: Sized;
    fn reset(&mut self);
    fn get_succinct_steps(&self) -> Vec<serde_json::Value>;
    fn get_full_steps(&self) -> Vec<serde_json::Value>;
    fn add_step(&mut self, step: MemoryStep);
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct LocalAgentMemory {
    pub steps: Vec<(Option<String>, MemoryStep)>, // (thread_id, step)
}

impl LocalAgentMemory {
    pub fn get_steps(&self, thread_id: Option<&str>) -> Vec<MemoryStep> {
        self.steps
            .iter()
            .filter(|(step_thread_id, _)| {
                thread_id.is_none_or(|tid| step_thread_id.as_ref().is_none_or(|stid| stid == tid))
            })
            .map(|(_, step)| step.clone())
            .collect()
    }

    pub fn add_step(&mut self, step: MemoryStep, thread_id: Option<&str>) {
        self.steps.push((thread_id.map(String::from), step));
    }

    pub fn get_steps_as_messages(
        &self,
        include_planning: bool,
        include_task: bool,
    ) -> Vec<Message> {
        self.steps
            .iter()
            .flat_map(|(_, step)| step.to_messages(!include_planning, include_task))
            .collect()
    }
}

impl AgentMemory for LocalAgentMemory {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn new() -> Self {
        Self { steps: Vec::new() }
    }

    fn reset(&mut self) {
        self.steps.clear();
    }

    fn get_succinct_steps(&self) -> Vec<serde_json::Value> {
        self.steps
            .iter()
            .map(|(_, step)| {
                let mut value = serde_json::to_value(step).unwrap();
                if let serde_json::Value::Object(obj) = &mut value {
                    obj.remove("model_input_messages");
                }
                value
            })
            .collect()
    }

    fn get_full_steps(&self) -> Vec<serde_json::Value> {
        self.steps
            .iter()
            .map(|(_, step)| serde_json::to_value(step).unwrap())
            .collect()
    }

    fn add_step(&mut self, step: MemoryStep) {
        self.steps.push((None, step));
    }
}

impl MemoryStep {
    pub fn to_messages(&self, summary_mode: bool, show_model_input_messages: bool) -> Vec<Message> {
        match self {
            MemoryStep::Task(task_step) => {
                vec![Message {
                    role: MessageRole::User,
                    name: Some("user".to_string()),
                    content: vec![MessageContent {
                        content_type: "text".to_string(),
                        text: Some(task_step.task.clone()),
                        image: None,
                    }],
                }]
            }
            MemoryStep::Planning(planning_step) if !summary_mode => {
                let mut messages = vec![planning_step.model_output_message_facts.clone()];
                messages.push(planning_step.model_output_message_plan.clone());
                messages
            }
            MemoryStep::Action(step) => {
                let mut messages = Vec::new();

                if show_model_input_messages {
                    if let Some(input_messages) = &step.model_input_messages {
                        messages.extend(input_messages.clone());
                    }
                }

                if let Some(output) = &step.model_output {
                    messages.push(Message {
                        role: MessageRole::Assistant,
                        name: None,
                        content: vec![MessageContent {
                            content_type: "text".to_string(),
                            text: Some(output.clone()),
                            image: None,
                        }],
                    });
                }

                messages
            }
            MemoryStep::System(system_step) => {
                vec![Message {
                    role: MessageRole::System,
                    name: None,
                    content: vec![MessageContent {
                        content_type: "text".to_string(),
                        text: Some(system_step.system_prompt.clone()),
                        image: None,
                    }],
                }]
            }
            _ => Vec::new(),
        }
    }
}
