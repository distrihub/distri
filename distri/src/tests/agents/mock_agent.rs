use std::sync::Arc;
use crate::{
    coordinator::CoordinatorContext,
    error::AgentError,
    memory::TaskStep,
    types::BaseAgent,
};

/// MockAgent that demonstrates custom agent behavior with pre/post execution hooks
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
        self.pre_execution_called.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn was_post_execution_called(&self) -> bool {
        self.post_execution_called.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn get_execution_log(&self) -> Vec<String> {
        self.execution_log.lock().unwrap().clone()
    }

    #[allow(dead_code)]
    pub fn reset(&self) {
        self.pre_execution_called.store(false, std::sync::atomic::Ordering::Relaxed);
        self.post_execution_called.store(false, std::sync::atomic::Ordering::Relaxed);
        self.execution_log.lock().unwrap().clear();
    }
}

#[async_trait::async_trait]
impl BaseAgent for MockAgent {
    async fn pre_execution(
        &self,
        agent_id: &str,
        task: &TaskStep,
        params: Option<&serde_json::Value>,
        context: Arc<CoordinatorContext>,
    ) -> Result<(), AgentError> {
        self.pre_execution_called.store(true, std::sync::atomic::Ordering::Relaxed);
        
        let log_entry = format!(
            "[{}] PRE-EXECUTION: agent_id={}, task={}, thread_id={}, params={:?}",
            self.name,
            agent_id,
            task.task,
            context.thread_id,
            params
        );
        
        self.execution_log.lock().unwrap().push(log_entry.clone());
        tracing::info!("{}", log_entry);
        
        // Simulate some async work
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        
        Ok(())
    }

    async fn post_execution(
        &self,
        agent_id: &str,
        task: &TaskStep,
        params: Option<&serde_json::Value>,
        context: Arc<CoordinatorContext>,
        result: &Result<String, AgentError>,
    ) -> Result<(), AgentError> {
        self.post_execution_called.store(true, std::sync::atomic::Ordering::Relaxed);
        
        let result_summary = match result {
            Ok(response) => format!("SUCCESS ({}chars)", response.len()),
            Err(e) => format!("ERROR: {}", e),
        };
        
        let log_entry = format!(
            "[{}] POST-EXECUTION: agent_id={}, task={}, thread_id={}, result={}, params={:?}",
            self.name,
            agent_id,
            task.task,
            context.thread_id,
            result_summary,
            params
        );
        
        self.execution_log.lock().unwrap().push(log_entry.clone());
        tracing::info!("{}", log_entry);
        
        // Simulate some async work
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A mock agent that can simulate failures
#[derive(Debug)]
pub struct FailingMockAgent {
    pub fail_pre: bool,
    pub fail_post: bool,
}

impl FailingMockAgent {
    pub fn new(fail_pre: bool, fail_post: bool) -> Self {
        Self { fail_pre, fail_post }
    }
}

#[async_trait::async_trait]
impl BaseAgent for FailingMockAgent {
    async fn pre_execution(
        &self,
        agent_id: &str,
        _task: &TaskStep,
        _params: Option<&serde_json::Value>,
        _context: Arc<CoordinatorContext>,
    ) -> Result<(), AgentError> {
        if self.fail_pre {
            return Err(AgentError::ToolExecution(format!(
                "Mock pre-execution failure for agent: {}", 
                agent_id
            )));
        }
        Ok(())
    }

    async fn post_execution(
        &self,
        agent_id: &str,
        _task: &TaskStep,
        _params: Option<&serde_json::Value>,
        _context: Arc<CoordinatorContext>,
        _result: &Result<String, AgentError>,
    ) -> Result<(), AgentError> {
        if self.fail_post {
            return Err(AgentError::ToolExecution(format!(
                "Mock post-execution failure for agent: {}", 
                agent_id
            )));
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}