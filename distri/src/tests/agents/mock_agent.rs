use crate::error::AgentError;

/// MockAgent that demonstrates custom agent behavior using the step() function
#[derive(Debug)]
pub struct MockAgent {
    pub name: String,
    pub pre_execution_called: std::sync::atomic::AtomicBool,
    pub post_execution_called: std::sync::atomic::AtomicBool,
    pub execution_log: std::sync::Mutex<Vec<String>>,
}

impl MockAgent {
    pub fn new(name: String) -> Self {
        Self {
            name,
            pre_execution_called: std::sync::atomic::AtomicBool::new(false),
            post_execution_called: std::sync::atomic::AtomicBool::new(false),
            execution_log: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn was_pre_execution_called(&self) -> bool {
        self.pre_execution_called
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn was_post_execution_called(&self) -> bool {
        self.post_execution_called
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn get_execution_log(&self) -> Vec<String> {
        self.execution_log.lock().unwrap().clone()
    }

    #[allow(dead_code)]
    pub fn reset(&self) {
        self.pre_execution_called
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.post_execution_called
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.execution_log.lock().unwrap().clear();
    }
}

#[async_trait::async_trait]
impl CustomAgent for MockAgent {
    async fn step(&self, context: &AgentExecutionContext) -> Result<String, AgentError> {
        // Mark pre-execution as called
        self.pre_execution_called
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let log_entry = format!(
            "[{}] STEP-EXECUTION: agent_id={}, task={}, thread_id={}, params={:?}",
            self.name,
            context.agent_id,
            context.task.task,
            context.coordinator_context.thread_id,
            context.params
        );

        self.execution_log.lock().unwrap().push(log_entry.clone());
        tracing::info!("{}", log_entry);

        // Simulate some async work
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Mark post-execution as called
        self.post_execution_called
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let completion_log = format!(
            "[{}] STEP-COMPLETED: agent_id={}",
            self.name, context.agent_id
        );

        self.execution_log
            .lock()
            .unwrap()
            .push(completion_log.clone());
        tracing::info!("{}", completion_log);

        Ok(format!("MockAgent {} executed successfully", self.name))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A mock agent that can simulate failures
#[derive(Debug)]
pub struct FailingMockAgent {
    pub fail_in_step: bool,
}

impl FailingMockAgent {
    pub fn new(fail_in_step: bool) -> Self {
        Self { fail_in_step }
    }
}

#[async_trait::async_trait]
impl CustomAgent for FailingMockAgent {
    async fn step(&self, context: &AgentExecutionContext) -> Result<String, AgentError> {
        if self.fail_in_step {
            return Err(AgentError::ToolExecution(format!(
                "Mock step execution failure for agent: {}",
                context.agent_id
            )));
        }
        Ok(format!("FailingMockAgent executed successfully"))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
