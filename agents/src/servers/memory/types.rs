use serde::{Deserialize, Serialize};

use crate::types::ToolCall;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    Assistant,
    User,
    ToolResponse,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MessageContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Message {
    pub role: MessageRole,
    pub content: Vec<MessageContent>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentError {
    pub message: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum MemoryStep {
    Action(ActionStep),
    Planning(PlanningStep),
    Task(TaskStep),
    SystemPrompt(SystemPromptStep),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemPromptStep {
    pub system_prompt: String,
}

pub trait AgentMemory: Send + Sync {
    fn new(system_prompt: String) -> Self
    where
        Self: Sized;
    fn reset(&mut self);
    fn get_succinct_steps(&self) -> Vec<serde_json::Value>;
    fn get_full_steps(&self) -> Vec<serde_json::Value>;
    fn add_step(&mut self, step: MemoryStep);
    fn get_system_prompt(&self) -> &SystemPromptStep;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LocalAgentMemory {
    pub system_prompt: SystemPromptStep,
    pub steps: Vec<MemoryStep>,
}

impl AgentMemory for LocalAgentMemory {
    fn new(system_prompt: String) -> Self {
        Self {
            system_prompt: SystemPromptStep { system_prompt },
            steps: Vec::new(),
        }
    }

    fn reset(&mut self) {
        self.steps.clear();
    }

    fn get_succinct_steps(&self) -> Vec<serde_json::Value> {
        self.steps
            .iter()
            .map(|step| {
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
            .map(|step| serde_json::to_value(step).unwrap())
            .collect()
    }

    fn add_step(&mut self, step: MemoryStep) {
        self.steps.push(step);
    }

    fn get_system_prompt(&self) -> &SystemPromptStep {
        &self.system_prompt
    }
}

impl MemoryStep {
    pub fn to_messages(&self, summary_mode: bool, show_model_input_messages: bool) -> Vec<Message> {
        match self {
            MemoryStep::Action(step) => {
                let mut messages = Vec::new();

                if show_model_input_messages {
                    if let Some(input_messages) = &step.model_input_messages {
                        messages.extend(input_messages.clone());
                    }
                }

                if !summary_mode {
                    if let Some(output) = &step.model_output {
                        messages.push(Message {
                            role: MessageRole::Assistant,
                            content: vec![MessageContent {
                                content_type: "text".to_string(),
                                text: Some(output.trim().to_string()),
                                image: None,
                            }],
                        });
                    }
                }

                if let Some(tool_calls) = &step.tool_calls {
                    messages.push(Message {
                        role: MessageRole::Assistant,
                        content: vec![MessageContent {
                            content_type: "text".to_string(),
                            text: Some(format!("Calling tools:\n{:?}", tool_calls)),
                            image: None,
                        }],
                    });
                }

                if let Some(observations) = &step.observations {
                    let tool_id = step
                        .tool_calls
                        .as_ref()
                        .and_then(|calls| calls.first())
                        .map(|call| call.tool_id.clone())
                        .unwrap_or_default();

                    messages.push(Message {
                        role: MessageRole::ToolResponse,
                        content: vec![MessageContent {
                            content_type: "text".to_string(),
                            text: Some(format!(
                                "Call id: {}\nObservation:\n{}",
                                tool_id, observations
                            )),
                            image: None,
                        }],
                    });
                }

                if let Some(error) = &step.error {
                    let error_message = format!(
                        "Error:\n{}\nNow let's retry: take care not to repeat previous errors! If you have retried several times, try a completely different approach.\n",
                        error.message
                    );

                    let message_content = if let Some(tool_calls) = &step.tool_calls {
                        if let Some(first_call) = tool_calls.first() {
                            format!("Call id: {}\n{}", first_call.tool_id, error_message)
                        } else {
                            error_message
                        }
                    } else {
                        error_message
                    };

                    messages.push(Message {
                        role: MessageRole::ToolResponse,
                        content: vec![MessageContent {
                            content_type: "text".to_string(),
                            text: Some(message_content),
                            image: None,
                        }],
                    });
                }

                if let Some(images) = &step.observations_images {
                    let mut content = vec![MessageContent {
                        content_type: "text".to_string(),
                        text: Some("Here are the observed images:".to_string()),
                        image: None,
                    }];

                    content.extend(images.iter().map(|image| MessageContent {
                        content_type: "image".to_string(),
                        text: None,
                        image: Some(image.clone()),
                    }));

                    messages.push(Message {
                        role: MessageRole::User,
                        content,
                    });
                }

                messages
            }
            MemoryStep::Planning(step) => {
                let mut messages = Vec::new();

                messages.push(Message {
                    role: MessageRole::Assistant,
                    content: vec![MessageContent {
                        content_type: "text".to_string(),
                        text: Some(format!("[FACTS LIST]:\n{}", step.facts.trim())),
                        image: None,
                    }],
                });

                if !summary_mode {
                    messages.push(Message {
                        role: MessageRole::Assistant,
                        content: vec![MessageContent {
                            content_type: "text".to_string(),
                            text: Some(format!("[PLAN]:\n{}", step.plan.trim())),
                            image: None,
                        }],
                    });
                }

                messages
            }
            MemoryStep::Task(step) => {
                let mut content = vec![MessageContent {
                    content_type: "text".to_string(),
                    text: Some(format!("New task:\n{}", step.task)),
                    image: None,
                }];

                if let Some(images) = &step.task_images {
                    content.extend(images.iter().map(|image| MessageContent {
                        content_type: "image".to_string(),
                        text: None,
                        image: Some(image.clone()),
                    }));
                }

                vec![Message {
                    role: MessageRole::User,
                    content,
                }]
            }
            MemoryStep::SystemPrompt(step) => {
                if summary_mode {
                    return vec![];
                }

                vec![Message {
                    role: MessageRole::System,
                    content: vec![MessageContent {
                        content_type: "text".to_string(),
                        text: Some(step.system_prompt.trim().to_string()),
                        image: None,
                    }],
                }]
            }
        }
    }
}
