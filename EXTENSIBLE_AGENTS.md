# Extensible Agents in Distri

This document describes the extensible agent system in Distri, which allows developers to create custom agents that extend the standard agent functionality with custom pre and post processing steps.

## Overview

The agent system in Distri has been refactored to be more extensible. The core functionality is now provided by `StandardAgent` (formerly `DefaultAgent`), which implements the full LLM-based agent behavior with extensible hooks.

## Architecture

### BaseAgent Trait

The `BaseAgent` trait defines the interface that all agents must implement:

```rust
#[async_trait::async_trait]
pub trait BaseAgent: Send + Sync + std::fmt::Debug {
    // Core methods
    async fn invoke(&self, task: TaskStep, params: Option<serde_json::Value>, 
                   context: Arc<ExecutorContext>, event_tx: Option<mpsc::Sender<AgentEvent>>) 
                   -> Result<String, AgentError>;
    
    async fn invoke_stream(&self, task: TaskStep, params: Option<serde_json::Value>,
                          context: Arc<ExecutorContext>, event_tx: mpsc::Sender<AgentEvent>) 
                          -> Result<(), AgentError>;

    // Extensible hook methods with default implementations
    async fn after_task_step(&self, task: TaskStep, context: Arc<ExecutorContext>) -> Result<(), AgentError>;
    async fn before_llm_step(&self, messages: &[Message], params: &Option<serde_json::Value>, 
                            context: Arc<ExecutorContext>) -> Result<Vec<Message>, AgentError>;
    async fn before_tool_calls(&self, tool_calls: &[ToolCall], context: Arc<ExecutorContext>) 
                              -> Result<Vec<ToolCall>, AgentError>;
    async fn after_tool_calls(&self, tool_responses: &[String], context: Arc<ExecutorContext>) 
                             -> Result<(), AgentError>;
    async fn after_finish(&self, content: &str, context: Arc<ExecutorContext>) -> Result<(), AgentError>;

    // Required metadata methods
    fn clone_box(&self) -> Box<dyn BaseAgent>;
    fn get_name(&self) -> &str;
    fn get_description(&self) -> &str;
    fn get_definition(&self) -> AgentDefinition;
    fn get_tools(&self) -> Vec<ServerTools>;
}
```

### StandardAgent

`StandardAgent` is the default implementation that provides the full agent functionality. It properly calls all the hook methods during execution, making it easy to extend.

### Hook Methods

The following hook methods are available for customization:

1. **`after_task_step`**: Called after the task is stored but before LLM processing begins
2. **`before_llm_step`**: Called before each LLM call, allows message modification
3. **`before_tool_calls`**: Called before tool execution, allows tool call modification  
4. **`after_tool_calls`**: Called after tool execution with the responses
5. **`after_finish`**: Called when the agent completes its task

## Creating Custom Agents

### Method 1: Composition (Recommended)

The recommended approach is to compose with `StandardAgent` and override specific hook methods:

```rust
#[derive(Clone)]
pub struct LoggingAgent {
    inner: StandardAgent,
}

impl LoggingAgent {
    pub fn new(
        definition: AgentDefinition,
        server_tools: Vec<ServerTools>,
        coordinator: Arc<AgentExecutor>,
        context: Arc<ExecutorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Self {
        let inner = StandardAgent::new(definition, server_tools, coordinator, context, session_store);
        Self { inner }
    }
}

#[async_trait::async_trait]
impl BaseAgent for LoggingAgent {
    // Delegate core methods to inner
    async fn invoke(&self, task: TaskStep, params: Option<serde_json::Value>,
                   context: Arc<ExecutorContext>, event_tx: Option<mpsc::Sender<AgentEvent>>) 
                   -> Result<String, AgentError> {
        self.inner.invoke(task, params, context, event_tx).await
    }

    // Delegate metadata methods
    fn get_name(&self) -> &str { self.inner.get_name() }
    // ... other delegate methods

    // Override hook methods for custom behavior
    async fn before_llm_step(&self, messages: &[Message], params: &Option<serde_json::Value>,
                            context: Arc<ExecutorContext>) -> Result<Vec<Message>, AgentError> {
        info!("🤖 LoggingAgent: About to call LLM with {} messages", messages.len());
        
        // Add custom system message
        let mut enhanced_messages = messages.to_vec();
        enhanced_messages.insert(0, Message {
            role: MessageRole::System,
            name: Some("logging_agent".to_string()),
            content: vec![MessageContent {
                content_type: "text".to_string(),
                text: Some("You are an enhanced agent with logging capabilities.".to_string()),
                image: None,
            }],
            tool_calls: vec![],
        });

        // Call parent implementation
        self.inner.before_llm_step(&enhanced_messages, params, context).await
    }
}
```

## Example Agents

### LoggingAgent

The `LoggingAgent` example demonstrates:
- Enhanced logging throughout the execution process
- Adding custom system messages to modify LLM behavior
- Hook method override patterns

### FilteringAgent

The `FilteringAgent` example demonstrates:
- Content filtering for both input and output
- Message preprocessing
- Response post-processing

## CLI Improvements

The CLI has been improved to better expose agent functionality:

### Commands

1. **`run`** - Run agent in interactive chat mode or execute a single task
   - `--agent <name>` - Specify agent (uses first agent if not provided)
   - `--background` - Run single task non-interactively 
   - `--task <task>` - Task to execute (required with --background)

2. **`start-server`** - Start the A2A server (renamed from `serve`)
   - `--host <host>` - Server host (default: 127.0.0.1)
   - `--port <port>` - Server port (default: 8080)

3. **`list`** - List available agents
4. **`list-tools`** - List available tools
5. **`proxy`** - Start MCP proxy server
6. **`config-schema`** - Generate configuration schema

### Usage Examples

```bash
# Interactive chat with first agent
distri run

# Interactive chat with specific agent
distri run --agent my-agent

# Execute single task in background
distri run --agent my-agent --background --task "What is the weather?"

# Start A2A server
distri start-server --port 8080
```

## Testing

Tests are provided in `distri/src/tests/extensible_agent_test.rs` that demonstrate:

1. **LoggingAgent functionality** - Tests custom logging behavior
2. **FilteringAgent functionality** - Tests content filtering
3. **Hook execution** - Verifies that all hooks are properly called

Run tests with:
```bash
cargo test extensible_agent_test
```

## Migration Guide

### From DefaultAgent to StandardAgent

If you were using `DefaultAgent` directly:

```rust
// Old way
let agent = DefaultAgent::new(definition, tools, coordinator, context, session_store);

// New way (DefaultAgent is now an alias to StandardAgent)
let agent = StandardAgent::new(definition, tools, coordinator, context, session_store);
// or continue using DefaultAgent (alias)
let agent = DefaultAgent::new(definition, tools, coordinator, context, session_store);
```

### Creating Custom Agents

Instead of implementing everything from scratch, extend `StandardAgent`:

```rust
// Old way - implementing everything
impl BaseAgent for MyAgent {
    async fn invoke(&self, ...) -> Result<String, AgentError> {
        // Implement entire agent logic
    }
}

// New way - extend StandardAgent
#[derive(Clone)]
pub struct MyAgent {
    inner: StandardAgent,
}

impl BaseAgent for MyAgent {
    // Delegate core methods to inner
    async fn invoke(&self, ...) -> Result<String, AgentError> {
        self.inner.invoke(task, params, context, event_tx).await
    }
    
    // Override only the hooks you need
    async fn before_llm_step(&self, ...) -> Result<Vec<Message>, AgentError> {
        // Custom preprocessing
        // ...
        self.inner.before_llm_step(messages, params, context).await
    }
}
```

## Best Practices

1. **Use Composition**: Prefer composing with `StandardAgent` over reimplementing the entire agent
2. **Call Parent Methods**: Always call the parent implementation in your hook overrides unless you have a specific reason not to
3. **Error Handling**: Proper error handling in hook methods to avoid breaking the execution flow
4. **Logging**: Use structured logging to make debugging easier
5. **Testing**: Test your custom agents thoroughly, especially the hook behaviors

## Future Enhancements

Planned improvements to the extensible agent system:

1. **Plugin System**: Dynamic loading of agent extensions
2. **Configuration-Based Extension**: Define custom behavior via configuration
3. **Middleware Pattern**: Composable middleware for common agent extensions
4. **Agent Chaining**: Easy composition of multiple agent behaviors