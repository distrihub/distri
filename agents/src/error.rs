use thiserror::Error;

#[derive(Error, Debug)]
pub enum AgentError {
    #[error("LLMError: {0}")]
    LLMError(String),
    #[error("MCP service error: {0}")]
    McpService(String),
    #[error("Tool execution error: {0}")]
    ToolExecution(String),
}
