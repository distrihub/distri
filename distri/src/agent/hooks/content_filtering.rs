use crate::{
    agent::{AgentHooks, StepResult},
    error::AgentError,
};
use tracing::info;

/// Hooks implementation for content filtering capability
#[derive(Clone, Debug)]
pub struct ContentFilteringHooks {
    banned_words: Vec<String>,
}

impl ContentFilteringHooks {
    pub fn new(banned_words: Vec<String>) -> Self {
        Self { banned_words }
    }

    fn filter_content(&self, content: &str) -> String {
        let mut filtered = content.to_string();
        for word in &self.banned_words {
            let replacement = "*".repeat(word.len());
            filtered = filtered.replace(word, &replacement);
        }
        filtered
    }
}

#[async_trait::async_trait]
impl AgentHooks for ContentFilteringHooks {
    async fn after_finish(&self, step_result: StepResult) -> Result<StepResult, AgentError> {
        match step_result {
            StepResult::Finish(content) => {
                let filtered = self.filter_content(&content);
                info!(
                    "🔧 ContentFilteringHooks: Content filtered - original: {} chars, filtered: {} chars",
                    content.len(),
                    filtered.len()
                );
                Ok(StepResult::Finish(filtered))
            }
            _ => Ok(step_result),
        }
    }
}
