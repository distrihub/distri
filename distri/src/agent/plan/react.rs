// --- React Planner (stub) ---
pub struct ReactPlanner;

pub struct ReactPlan {
    pub steps: Vec<String>,
}

impl Plan for ReactPlan {
    fn format(&self) -> Vec<String> {
        self.steps.clone()
    }
    fn as_json(&self) -> Value {
        serde_json::json!({ "steps": self.steps })
    }
}

#[async_trait::async_trait]
impl Planner for ReactPlanner {
    async fn plan(
        &self,
        _message: &Message,
        _plan_config: &PlanConfig,
        _current_messages: &mut Vec<Message>,
        iteration: usize,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<PlanResult, AgentError> {
        if let Some(event_tx) = &event_tx {
            let _ = event_tx
                .send(AgentEvent {
                    thread_id: context.thread_id.clone(),
                    run_id: context.run_id.clone(),
                    event: AgentEventType::PlanStarted {
                        initial_plan: iteration == 0,
                    },
                })
                .await;
        }
        // Example REACT plan (stub):
        let steps = vec![
            "Thought: ...".to_string(),
            "Action: ...".to_string(),
            "Observation: ...".to_string(),
        ];
        if let Some(event_tx) = &event_tx {
            let _ = event_tx
                .send(AgentEvent {
                    thread_id: context.thread_id.clone(),
                    run_id: context.run_id.clone(),
                    event: AgentEventType::PlanFinished {
                        facts: "react facts stub".to_string(),
                        plan: format!("steps: {}", steps.join(", ")), // stub
                    },
                })
                .await;
        }
        let plan = ReactPlan { steps };
        let json = plan.as_json();
        Ok(PlanResult {
            plan: Box::new(plan),
            json,
        })
    }
    fn format(&self, plan: &dyn Plan) -> Vec<String> {
        plan.format()
    }
    async fn llm(
        &self,
        messages: &mut Vec<Message>,
        plan_config: &PlanConfig,
        context: Arc<ExecutorContext>,
        prompt: &str,
    ) -> Result<String, AgentError> {
        let planning_executor = LLMExecutor::new(
            crate::agent::reason::get_planning_definition(plan_config.model_settings.clone()),
            Arc::default(),
            context.clone(),
            None,
            Some("react_plan".to_string()),
        );
        let response = planning_executor.execute(messages).await;
        match response {
            Ok(response) => Ok(response.content.clone()),
            Err(e) => {
                tracing::error!("React planning execution failed: {}", e);
                Ok(format!("React planning execution failed: {}", e))
            }
        }
    }
}
