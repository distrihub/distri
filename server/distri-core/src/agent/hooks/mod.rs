use std::sync::Arc;

use crate::agent::types::AgentHooks;
use crate::AgentError;

pub mod inline;

/// Utility hook that fans out lifecycle events to multiple hook implementations.
#[derive(Debug)]
pub struct CombinedHooks {
    hooks: Vec<Arc<dyn AgentHooks>>,
}

impl CombinedHooks {
    pub fn new(hooks: Vec<Arc<dyn AgentHooks>>) -> Self {
        Self { hooks }
    }
}

#[async_trait::async_trait]
impl AgentHooks for CombinedHooks {
    async fn before_execute(
        &self,
        message: &mut crate::types::Message,
        context: Arc<crate::agent::types::ExecutorContext>,
    ) -> Result<(), AgentError> {
        for hook in &self.hooks {
            hook.before_execute(message, context.clone()).await?;
        }
        Ok(())
    }

    async fn on_plan_start(
        &self,
        message: &mut crate::types::Message,
        context: Arc<crate::agent::types::ExecutorContext>,
    ) -> Result<(), AgentError> {
        for hook in &self.hooks {
            hook.on_plan_start(message, context.clone()).await?;
        }
        Ok(())
    }

    async fn on_plan_end(
        &self,
        message: &mut crate::types::Message,
        context: Arc<crate::agent::types::ExecutorContext>,
        plan: &[distri_types::PlanStep],
    ) -> Result<(), AgentError> {
        for hook in &self.hooks {
            hook.on_plan_end(message, context.clone(), plan).await?;
        }
        Ok(())
    }

    async fn on_step_start(&self, step: &distri_types::PlanStep) -> Result<(), AgentError> {
        for hook in &self.hooks {
            hook.on_step_start(step).await?;
        }
        Ok(())
    }

    async fn on_step_end(
        &self,
        context: Arc<crate::agent::types::ExecutorContext>,
        step: &distri_types::PlanStep,
        result: &distri_types::ExecutionResult,
    ) -> Result<(), AgentError> {
        for hook in &self.hooks {
            hook.on_step_end(context.clone(), step, result).await?;
        }
        Ok(())
    }

    async fn on_halt(&self, reason: &str) -> Result<(), AgentError> {
        for hook in &self.hooks {
            hook.on_halt(reason).await?;
        }
        Ok(())
    }
}
