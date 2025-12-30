use serde::{Deserialize, Serialize};

/// Overrides for agent definition - only the most commonly overridden fields
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DefinitionOverrides {
    /// Override the model (e.g., "gpt-4o", "gpt-4.1-mini")
    pub model: Option<String>,
    /// Override the temperature
    pub temperature: Option<f32>,
    /// Override max tokens
    pub max_tokens: Option<u32>,
    /// Override max iterations
    pub max_iterations: Option<usize>,
    /// Override instructions
    pub instructions: Option<String>,

    /// Override browser usage flag
    pub use_browser: Option<bool>,
}

impl DefinitionOverrides {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_model(mut self, model: String) -> Self {
        self.model = Some(model);
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn with_max_iterations(mut self, max_iterations: usize) -> Self {
        self.max_iterations = Some(max_iterations);
        self
    }

    pub fn with_instructions(mut self, instructions: String) -> Self {
        self.instructions = Some(instructions);
        self
    }

    pub fn with_browser_enabled(mut self, enabled: bool) -> Self {
        self.use_browser = Some(enabled);
        self
    }
}
