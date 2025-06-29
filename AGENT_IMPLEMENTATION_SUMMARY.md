# Custom Agent Implementation Summary

## Overview

This implementation provides a powerful and flexible system for creating custom agents that can execute arbitrary code while having access to LLM capabilities, session management, and tool execution. The system moves beyond simple YAML configuration to allow full programmatic control over agent behavior.

## Key Components

### 1. AgentStore (`distri/src/agent_store.rs`)

The `AgentStore` is a centralized manager for all types of agents:

- **Local Agents**: YAML-configured agents
- **Remote Agents**: A2A (Agent-to-Agent) URLs  
- **Runnable Agents**: Custom code implementations

```rust
// Create an agent store
let agent_store = AgentStore::new();

// Register different types of agents
agent_store.register_local_agent(yaml_definition, tools).await?;
agent_store.register_remote_agent("remote_agent".to_string(), "http://url".to_string()).await?;
agent_store.register_runnable_agent(definition, custom_agent, tools).await?;
```

### 2. CustomAgent Trait

Custom agents implement the `CustomAgent` trait with a single `step()` method:

```rust
#[async_trait::async_trait]
pub trait CustomAgent: Send + Sync + std::fmt::Debug {
    /// Main execution step - custom agents implement their logic here
    async fn step(
        &self,
        context: &AgentExecutionContext,
    ) -> Result<String, AgentError>;

    /// Support for downcasting (useful for testing)
    fn as_any(&self) -> &dyn std::any::Any;
}
```

### 3. AgentExecutionContext

The execution context provides custom agents with access to:

```rust
pub struct AgentExecutionContext {
    pub agent_id: String,
    pub task: TaskStep,
    pub params: Option<serde_json::Value>,
    pub coordinator_context: Arc<CoordinatorContext>,
    pub llm_executor: LLMExecutorWrapper,
    pub session_writer: SessionWriter,
}
```

### 4. LLMExecutorWrapper

Provides custom agents with LLM capabilities:

```rust
impl LLMExecutorWrapper {
    /// Execute LLM with current messages from session
    pub async fn llm(&self, messages: Vec<Message>) -> Result<String, AgentError>;

    /// Execute LLM with streaming
    pub async fn llm_stream(
        &self,
        messages: Vec<Message>,
        event_tx: tokio::sync::mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError>;
}
```

### 5. SessionWriter

Allows custom agents to write data into session and retrieve messages:

```rust
impl SessionWriter {
    /// Write a memory step to the session
    pub async fn write_step(&self, step: MemoryStep) -> Result<(), AgentError>;

    /// Get current messages from session
    pub async fn get_messages(&self) -> Result<Vec<Message>, AgentError>;
}
```

## Example Implementation

### Basic Custom Agent

```rust
use crate::{
    agent_store::{AgentExecutionContext, CustomAgent},
    error::AgentError,
    memory::{MemoryStep, SystemStep},
    types::{Message, MessageContent, MessageRole},
};

#[derive(Debug)]
pub struct MyCustomAgent {
    pub name: String,
}

impl MyCustomAgent {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}

#[async_trait::async_trait]
impl CustomAgent for MyCustomAgent {
    async fn step(
        &self,
        context: &AgentExecutionContext,
    ) -> Result<String, AgentError> {
        // 1. Write custom preprocessing to session
        let preprocessing_step = MemoryStep::System(SystemStep {
            system_prompt: format!("Custom preprocessing by {}", self.name),
        });
        context.session_writer.write_step(preprocessing_step).await?;

        // 2. Get current messages and add task
        let mut messages = context.session_writer.get_messages().await?;
        let user_message = Message {
            role: MessageRole::User,
            name: Some("user".to_string()),
            content: vec![MessageContent {
                content_type: "text".to_string(),
                text: Some(context.task.task.clone()),
                image: None,
            }],
            tool_calls: vec![],
        };
        messages.push(user_message);

        // 3. Call LLM
        let llm_response = context.llm_executor.llm(messages).await?;

        // 4. Custom post-processing
        let result = format!(
            "Processed by {}: {}\n\nLLM Response: {}",
            self.name, "Custom logic completed", llm_response
        );

        Ok(result)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
```

### API-Enhanced Agent

```rust
#[derive(Debug)]
pub struct ApiAgent {
    pub name: String,
    pub api_calls: std::sync::Mutex<Vec<String>>,
}

impl ApiAgent {
    pub fn new(name: String) -> Self {
        Self {
            name,
            api_calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    async fn call_external_api(&self, endpoint: &str, data: &str) -> Result<String, AgentError> {
        // Simulate API call
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        let call = format!("API_CALL: {} with data: {}", endpoint, data);
        self.api_calls.lock().unwrap().push(call.clone());
        Ok(format!("{{\"status\": \"success\", \"response\": \"data from {}\"}}", endpoint))
    }
}

#[async_trait::async_trait]
impl CustomAgent for ApiAgent {
    async fn step(
        &self,
        context: &AgentExecutionContext,
    ) -> Result<String, AgentError> {
        // 1. Determine which API to call based on task
        let task_data = &context.task.task;
        let api_response = if task_data.contains("weather") {
            self.call_external_api("/api/weather", task_data).await?
        } else if task_data.contains("user") {
            self.call_external_api("/api/users", task_data).await?
        } else {
            self.call_external_api("/api/general", task_data).await?
        };

        // 2. Write API response to session
        let api_step = MemoryStep::System(SystemStep {
            system_prompt: format!("API Response: {}", api_response),
        });
        context.session_writer.write_step(api_step).await?;

        // 3. Create enriched context for LLM
        let mut messages = context.session_writer.get_messages().await?;
        let enriched_message = Message {
            role: MessageRole::User,
            name: Some("api_context".to_string()),
            content: vec![MessageContent {
                content_type: "text".to_string(),
                text: Some(format!("API Data: {}\n\nUser Request: {}", 
                    api_response, context.task.task)),
                image: None,
            }],
            tool_calls: vec![],
        };
        messages.push(enriched_message);

        // 4. Call LLM with enriched context
        let llm_response = context.llm_executor.llm(messages).await?;

        // 5. Return combined response
        Ok(format!("API-Enhanced Response:\n{}", llm_response))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
```

## Integration with Coordinator

### Initializing with AgentStore

```rust
use distri::{
    agent_store::AgentStore,
    coordinator::LocalCoordinator,
    types::AgentDefinition,
};

// Create agent store
let agent_store = AgentStore::new();

// Register custom agents
let custom_agent = MyCustomAgent::new("CustomProcessor".to_string());
let agent_def = AgentDefinition {
    name: "my_custom_agent".to_string(),
    description: "Custom agent with step-based execution".to_string(),
    // ... other configuration
};

agent_store.register_runnable_agent(
    agent_def,
    Box::new(custom_agent),
    vec![] // tools
).await?;

// Initialize coordinator with agent store
let coordinator = LocalCoordinator::new_with_agent_store(
    registry,
    tool_sessions,
    session_store,
    context,
    agent_store.clone(),
);
```

### Execution

```rust
// Execute through agent store (automatically routes to custom implementation)
let result = agent_store.execute_agent(
    "my_custom_agent",
    task,
    params,
    context,
    Arc::new(coordinator),
).await?;

// Or execute with streaming
agent_store.execute_agent_stream(
    "my_custom_agent",
    task,
    params,
    context,
    event_tx,
    Arc::new(coordinator),
).await?;
```

## Benefits

### 1. **Full Programmatic Control**
- Implement custom logic before, during, and after LLM calls
- Handle complex workflows and state management
- Integration with external APIs and services

### 2. **Session Management**
- Read and write to agent session
- Access conversation history
- Store custom context and state

### 3. **LLM Integration**
- Easy access to LLM capabilities through wrapper
- Support for streaming responses
- Tool execution integration

### 4. **Backward Compatibility**
- Existing YAML agents continue to work unchanged
- Gradual migration path from YAML to code

### 5. **Testing and Debugging**
- Easy to unit test custom agent logic
- Rich logging and debugging capabilities
- Downcasting support for test verification

## Architecture Benefits

### Separation of Concerns
- **AgentStore**: Manages agent registration and routing
- **CustomAgent**: Implements business logic
- **RunnableAgent**: Handles execution lifecycle
- **LocalCoordinator**: Coordinates with existing infrastructure

### Flexibility
- Support for different agent types (Local, Remote, Runnable)
- Easy to extend with new agent types
- Modular design allows independent development

### Performance
- Direct code execution (no YAML parsing overhead)
- Efficient session access
- Parallel execution support

## Testing

The implementation includes comprehensive tests demonstrating:

- Agent store creation and management
- Custom agent registration and execution
- Error handling and failure scenarios
- Integration with coordinator
- Parameter passing and context access

```bash
# Run tests
cargo test --lib tests::agents::step_agent_test
```

## Migration Guide

### From Simple YAML Agents
1. Keep existing YAML configuration for agent metadata
2. Implement `CustomAgent` trait for business logic
3. Register as runnable agent with existing definition

### From Hook-based Approach
1. Move pre/post execution logic into single `step()` method
2. Use session writer for state management
3. Call LLM explicitly when needed

This architecture provides the foundation for building sophisticated, programmable agents while maintaining the simplicity and power of the existing distri framework.