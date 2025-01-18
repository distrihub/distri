use thiserror::Error;

#[derive(Error, Debug)]
pub enum AgentError {
    #[error("OpenAI API error: {0}")]
    OpenAI(String),
    #[error("MCP service error: {0}")]
    McpService(String),
    #[error("Tool execution error: {0}")]
    ToolExecution(String),
}
