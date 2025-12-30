use serde_json::Value;
use std::sync::Arc;
use std::sync::OnceLock;

/// Trait for storing and retrieving step history keyed by task/thread identifiers.
pub trait SequenceStepStore: Send + Sync {
    fn record_step(&self, task_id: &str, step: Value);
    fn get_steps(&self, task_id: &str) -> Vec<Value>;
    fn count_steps(&self, task_id: &str) -> usize {
        self.get_steps(task_id).len()
    }
}

static GLOBAL_SEQUENCE_STORE: OnceLock<Arc<dyn SequenceStepStore>> = OnceLock::new();

/// Register a global sequence store (call during startup).
pub fn set_global_sequence_store(store: Arc<dyn SequenceStepStore>) {
    let _ = GLOBAL_SEQUENCE_STORE.set(store);
}

/// Get the global sequence store if one was registered.
pub fn get_global_sequence_store() -> Option<Arc<dyn SequenceStepStore>> {
    GLOBAL_SEQUENCE_STORE.get().cloned()
}
