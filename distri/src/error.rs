use thiserror::Error;

#[derive(Error, Debug)]
pub enum AgentError {
    #[error("LLMError: {0}")]
    LLMError(String),
    #[error("MCP service error: {0}")]
    McpService(String),
    #[error("Tool execution error: {0}")]
    ToolExecution(String),
    #[error("Session error: {0}")]
    Session(String),
    #[error("Parameters error: {0}")]
    Parameter(String),
    #[error("Not found: {0}")]
    NotFound(String),
}
