use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt::Display, sync::Arc};

use crate::{
    agent::ExecutorContext,
    llm::LLMExecutor,
    types::{Message, MessageMetadata, MessageRole, PlanConfig},
    AgentError,
};

mod default;

pub use default::{DefaultPlan, DefaultPlanner};

// System prompts and definitions as static includes
pub static PROMPTS: &str = include_str!("prompts.yaml");

pub trait Plan: Send + Sync {
    fn as_string(&self) -> String;
}

pub fn get_prompts() -> HashMap<String, String> {
    let prompts: HashMap<String, String> = serde_yaml::from_str(PROMPTS).unwrap();
    prompts
}

#[async_trait::async_trait]
pub trait Planner: Send + Sync {
    fn get_prompt(&self, strategy: &str) -> String {
        let prompts = get_prompts();
        prompts.get(strategy).unwrap().to_owned()
    }
    async fn plan(
        &self,
        message: &Message,
        plan_config: &PlanConfig,
        current_messages: &[Message],
        iteration: usize,
        context: Arc<ExecutorContext>,
        tools: &str,
    ) -> Result<Box<dyn Plan>, AgentError>;

    async fn llm(
        &self,
        messages: &[Message],
        plan_config: &PlanConfig,
        context: Arc<ExecutorContext>,
    ) -> Result<String, AgentError> {
        let planning_executor = LLMExecutor::new(
            crate::agent::reason::get_planning_definition(plan_config.model_settings.clone()),
            Arc::default(),
            context.clone(),
            None,
            Some("initial_plan".to_string()),
        );

        let response = planning_executor.execute(&messages).await;
        match response {
            Ok(response) => {
                // Extract just the content string
                let content = response.content.clone();
                Ok(content)
            }
            Err(e) => {
                tracing::error!("Planning execution failed: {}", e);
                Ok(format!("Planning execution failed: {}", e))
            }
        }
    }
}

pub fn get_planner(strategy: Option<&str>) -> Arc<dyn Planner> {
    match strategy {
        Some("default") => Arc::new(DefaultPlanner),
        Some("react") => todo!(),
        _ => Arc::new(DefaultPlanner),
    }
}

fn replace_variables(prompt: &str, variables: &HashMap<String, String>) -> String {
    let mut prompt = prompt.to_owned();
    for (key, value) in variables {
        prompt = prompt.replace(&format!("{{{}}}", key), value);
    }
    prompt
}

pub fn convert_messages_to_steps(messages: &[Message]) -> String {
    let mut steps = Vec::new();
    for message in messages {
        if let Some(metadata) = &message.metadata {
            match metadata {
                MessageMetadata::Plan { plan } => {
                    steps.push(Step::Plan(plan.clone()));
                }
                MessageMetadata::ToolResponse { result, .. } => {
                    steps.push(Step::Observation(result.clone()));
                }
                MessageMetadata::ToolCalls { tool_calls } => {
                    steps.push(Step::Action(
                        tool_calls
                            .iter()
                            .map(|tool_call| {
                                format!("{}: {}", tool_call.tool_name, tool_call.input)
                            })
                            .collect::<Vec<String>>()
                            .join("\n"),
                    ));
                }
                MessageMetadata::FinalResponse { .. } => {
                    steps.push(Step::Observation(message.as_text().unwrap_or_default()));
                }
            }
        }

        match message.role {
            MessageRole::Assistant => {
                steps.push(Step::Action(message.as_text().unwrap_or_default()));
            }
            MessageRole::System => continue,
            MessageRole::User => {
                steps.push(Step::Action(message.as_text().unwrap_or_default()));
            }
        }
    }
    steps
        .iter()
        .map(|step| step.to_string())
        .collect::<Vec<String>>()
        .join("\n")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "type", content = "value")]
pub enum Step {
    Plan(String),
    Thought(String),
    Action(String),
    Observation(String),
}

impl Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Step::Plan(plan) => write!(f, "Plan: {}", plan),
            Step::Thought(thought) => write!(f, "Thought: {}", thought),
            Step::Action(action) => write!(f, "Action: {}", action),
            Step::Observation(observation) => write!(f, "Observation: {}", observation),
        }
    }
}
