#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Agent not found: {0}")]
    AgentNotFound(String),
    #[error("Tool not found: {0}")]
    ToolNotFound(String),
    #[error("Tool execution failed: {0}")]
    ToolExecutionFailed(String),
    #[error("Tool execution error: {0}")]
    ToolExecution(String),
    #[error("Tool response processing failed: {0}")]
    ToolResponseProcessing(String),
    #[error("Authentication required: {0}")]
    AuthRequired(String),
    #[error("LLM execution failed: {0}")]
    LlmExecutionFailed(String),
    #[error("LLM error: {0}")]
    LLMError(String),
    #[error("{0}")]
    OpenAIError(#[from] async_openai::error::OpenAIError),
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),
    #[error("Session error: {0}")]
    Session(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Not implemented: {0}")]
    NotImplemented(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Unsupported file type: {0}")]
    UnsupportedFileType(String),
    #[error("Halt: {0}")]
    Halt(String),
    #[error("Planning error: {0}")]
    Planning(String),
    #[error("Parsing error: {0}")]
    Parsing(String),
    #[error("XML parsing failed, content: {0}, error: {1}")]
    XmlParsingFailed(String, String),
    #[error("JSON parsing failed, content: {0}, error: {1}")]
    JsonParsingFailed(String, String),
    #[error("Storage error: {0}")]
    Storage(String),
    #[error("Other error: {0}")]
    Other(String),
    #[error(transparent)]
    SerdeError(#[from] serde_json::Error),
    #[error("Execution error: {0}")]
    Execution(String),
    #[error("Workflow execution failed: {0}")]
    WorkflowExecutionFailed(String),
    #[error("Invalid workflow step: {0}")]
    InvalidWorkflowStep(String),
    #[error("Initialization error: {0}")]
    Initialization(String),
}
