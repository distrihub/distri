use std::{collections::HashMap, sync::Arc};

use crate::{
    agent::{
        plan::{convert_messages_to_steps, replace_variables, Plan, Planner},
        ExecutorContext,
    },
    types::{Message, PlanConfig},
    AgentError,
};

pub struct DefaultPlanner;

#[derive(serde::Deserialize, serde::Serialize)]
pub struct DefaultPlan {
    pub plan: String,
}

impl Plan for DefaultPlan {
    fn as_string(&self) -> String {
        self.plan.clone()
    }
}

#[async_trait::async_trait]
impl Planner for DefaultPlanner {
    async fn plan(
        &self,
        message: &Message,
        plan_config: &PlanConfig,
        current_messages: &[Message],
        iteration: usize,
        context: Arc<ExecutorContext>,
        tools: &str,
    ) -> Result<Box<dyn Plan>, AgentError> {
        let plan = if iteration == 0 {
            self.initial_plan(message, plan_config, tools, context.clone())
                .await?
        } else {
            self.update_plan(
                plan_config,
                current_messages,
                tools,
                iteration,
                context.clone(),
            )
            .await?
        };

        Ok(Box::new(plan) as Box<dyn Plan>)
    }
}

impl DefaultPlanner {
    async fn initial_plan(
        &self,
        message: &Message,
        plan_config: &PlanConfig,
        tools: &str,
        context: Arc<ExecutorContext>,
    ) -> Result<DefaultPlan, AgentError> {
        let prompt = self.get_prompt("default_initial");
        let prompt = replace_variables(
            &prompt,
            &HashMap::from([("tools".to_string(), tools.to_string())]),
        );

        let system_message = Message::system(prompt, Some("plan".to_string()));
        let messages = vec![system_message, message.clone()];
        let plan = self.llm(&messages, plan_config, context).await?;
        Ok(DefaultPlan { plan })
    }
    async fn update_plan(
        &self,
        plan_config: &PlanConfig,
        current_messages: &[Message],
        tools: &str,
        iteration: usize,
        context: Arc<ExecutorContext>,
    ) -> Result<DefaultPlan, AgentError> {
        let prompt = self.get_prompt("default_update");
        let remaining_steps = plan_config.max_iterations.unwrap_or(10) - iteration as usize;
        let prompt = replace_variables(
            &prompt,
            &HashMap::from([
                ("tools".to_string(), tools.to_string()),
                ("remaining_steps".to_string(), remaining_steps.to_string()),
            ]),
        );

        let steps = convert_messages_to_steps(current_messages);
        let messages = vec![
            Message::system(prompt, Some("plan".to_string())),
            Message::user(steps, Some("plan".to_string())),
        ];

        let plan = self.llm(&messages, plan_config, context).await?;
        Ok(DefaultPlan { plan })
    }
}
