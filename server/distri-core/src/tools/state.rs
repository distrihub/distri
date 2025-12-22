use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};

use crate::AgentError;

#[derive(Debug, Clone, Default)]
pub struct AgentExecutorState {
    pub completed: Arc<AtomicBool>,
    pub observations: Arc<RwLock<Vec<String>>>,
}

impl AgentExecutorState {
    pub fn add_observation(&self, observation: String) -> Result<(), AgentError> {
        let mut observations = self
            .observations
            .write()
            .map_err(|_| AgentError::Other("Failed to write to observations".to_string()))?;
        observations.push(observation);
        Ok(())
    }

    pub fn set_completed(&self, completed: bool) {
        self.completed.store(completed, Ordering::Release);
    }

    pub fn is_completed(&self) -> bool {
        self.completed.load(Ordering::Acquire)
    }

    pub fn get_observations(&self) -> Result<Vec<String>, AgentError> {
        let guard = self
            .observations
            .read()
            .map_err(|_| AgentError::Other("Failed to read observations".to_string()))?;
        Ok(guard.clone())
    }
}
