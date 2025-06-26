# Distri Programmable Agents Interface

The Distri Programmable Agents Interface provides a powerful way to create agents programmatically in Rust, similar to the Google ADK (Android Development Kit) pattern. This interface allows you to implement agents with custom logic while maintaining compatibility with the existing YAML-based configuration system.

## Overview

The programmable interface consists of several key components:

- **`Agent` trait**: The core interface that all programmable agents must implement
- **`AgentBuilder`**: A builder pattern for easy agent creation  
- **`ProgrammableAgent`**: A concrete implementation of the Agent trait
- **`AgentContext`**: Context information provided to agents during execution
- **`AgentResponse`**: The response structure returned by agents

## Key Features

✅ **Dual Support**: Both YAML-defined and programmatically-defined agents  
✅ **Builder Pattern**: Easy agent creation with fluent API  
✅ **Stateful Agents**: Agents can maintain internal state between invocations  
✅ **Custom Logic**: Full control over agent behavior in Rust code  
✅ **Tool Integration**: Support for MCP tools and custom tools  
✅ **Artifact Generation**: Agents can produce files, reports, and other artifacts  
✅ **Memory Integration**: Access to conversation history and context  

## Basic Usage

### 1. Implementing the Agent Trait

```rust
use distri::types::{Agent, AgentDefinition, AgentContext, AgentResponse, TaskStep};
use distri::error::AgentError;

struct MyCustomAgent {
    definition: AgentDefinition,
    // Add any state you need
    counter: u32,
}

#[async_trait::async_trait]
impl Agent for MyCustomAgent {
    fn definition(&self) -> &AgentDefinition {
        &self.definition
    }

    async fn invoke(
        &mut self,
        task: TaskStep,
        context: AgentContext,
        params: Option<serde_json::Value>,
    ) -> Result<AgentResponse, AgentError> {
        // Your custom logic here
        let response = format!("Processed: {}", task.task);
        Ok(AgentResponse::text(response))
    }
}
```

### 2. Using the Builder Pattern

```rust
use distri::types::ProgrammableAgent;

let agent = ProgrammableAgent::builder("my_agent")
    .description("A simple example agent")
    .system_prompt("You are a helpful assistant")
    .handler(|task, context| async move {
        let result = format!("Hello! You asked: {}", task.task);
        Ok(AgentResponse::text(result))
    })
    .build();
```

### 3. Registering and Using Agents

```rust
use distri::coordinator::{LocalCoordinator, CoordinatorContext};
use distri::servers::registry::ServerRegistry;

// Initialize coordinator
let registry = Arc::new(RwLock::new(ServerRegistry::new()));
let context = Arc::new(CoordinatorContext::default());
let coordinator = Arc::new(LocalCoordinator::new(registry, None, None, context));

// Register a programmable agent
let my_agent = Box::new(MyCustomAgent::new());
coordinator.register_programmable_agent(my_agent).await?;

// Execute the agent
let result = coordinator.execute(
    "my_agent",
    TaskStep {
        task: "Hello, world!".to_string(),
        task_images: None,
    },
    None,
).await?;

println!("Agent response: {}", result);
```

## Agent Types and Examples

### 1. Simple Function-Based Agent

Perfect for stateless operations:

```rust
let text_processor = ProgrammableAgent::builder("text_processor")
    .description("Processes text in various ways")
    .handler(|task, _context| async move {
        let text = &task.task;
        let result = if text.contains("uppercase") {
            text.to_uppercase()
        } else if text.contains("reverse") {
            text.chars().rev().collect()
        } else {
            format!("Processed: {}", text)
        };
        Ok(AgentResponse::text(result))
    })
    .build();
```

### 2. Stateful Agent with Internal State

Maintains state between invocations:

```rust
struct CounterAgent {
    definition: AgentDefinition,
    counter: Arc<AtomicU32>,
}

impl CounterAgent {
    fn new() -> Self {
        Self {
            definition: AgentDefinition {
                name: "counter".to_string(),
                description: "A stateful counter agent".to_string(),
                // ... other fields
            },
            counter: Arc::new(AtomicU32::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Agent for CounterAgent {
    async fn invoke(&mut self, task: TaskStep, _context: AgentContext, _params: Option<serde_json::Value>) -> Result<AgentResponse, AgentError> {
        match task.task.as_str() {
            "increment" => {
                let new_value = self.counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                Ok(AgentResponse::text(format!("Counter: {}", new_value)))
            },
            "get" => {
                let value = self.counter.load(std::sync::atomic::Ordering::SeqCst);
                Ok(AgentResponse::text(format!("Counter: {}", value)))
            },
            _ => Ok(AgentResponse::text("Unknown command".to_string())),
        }
    }
}
```

### 3. Agent with Artifacts

Generate files, reports, and other outputs:

```rust
struct ReportAgent {
    definition: AgentDefinition,
}

#[async_trait::async_trait]
impl Agent for ReportAgent {
    async fn invoke(&mut self, task: TaskStep, context: AgentContext, _params: Option<serde_json::Value>) -> Result<AgentResponse, AgentError> {
        // Generate report data
        let report_data = generate_report(&task.task);
        
        // Create artifacts
        let artifacts = vec![
            Artifact {
                id: uuid::Uuid::new_v4().to_string(),
                name: "report.json".to_string(),
                content_type: "application/json".to_string(),
                content: serde_json::to_string_pretty(&report_data)?,
                metadata: Some(serde_json::json!({"generated_at": chrono::Utc::now()})),
            },
        ];

        Ok(AgentResponse::text("Report generated successfully")
            .with_artifacts(artifacts))
    }
}
```

### 4. Agent with Tool Integration

Use existing MCP tools or define custom tools:

```rust
let tool_agent = ProgrammableAgent::builder("tool_agent")
    .description("An agent that uses tools")
    .with_tools(vec![
        McpDefinition {
            name: "filesystem".to_string(),
            r#type: McpServerType::Tool,
            filter: ToolsFilter::All,
        }
    ])
    .handler(|task, context| async move {
        // Use tools through the agent handle
        if let Some(handle) = context.agent_handle {
            let tool_result = handle.execute_tool(ToolCall {
                tool_id: "file_read".to_string(),
                tool_name: "read_file".to_string(),
                input: serde_json::to_string(&json!({"path": task.task}))?,
            }).await?;
            
            Ok(AgentResponse::text(format!("File content: {}", tool_result)))
        } else {
            Ok(AgentResponse::text("No tools available".to_string()))
        }
    })
    .build();
```

## Agent Context

The `AgentContext` provides important information during execution:

```rust
struct AgentContext {
    pub thread_id: String,        // Conversation thread ID
    pub run_id: String,           // Current execution run ID  
    pub user_id: Option<String>,  // User identifier
    pub verbose: bool,            // Verbose logging flag
    pub max_tokens: u32,          // Token limit
    pub max_iterations: i32,      // Iteration limit
    pub agent_handle: Option<AgentHandle>, // Handle for tool execution
    pub memory_store: Option<Arc<Box<dyn MemoryStore>>>, // Access to memory
}
```

## Agent Response

Agents return an `AgentResponse` with multiple types of content:

```rust
struct AgentResponse {
    pub content: String,                    // Primary response text
    pub artifacts: Vec<Artifact>,           // Generated files/documents
    pub tool_calls: Vec<ToolCall>,         // Tool calls to execute
    pub metadata: Option<serde_json::Value>, // Additional metadata
}

// Helper methods for building responses
AgentResponse::text("Simple text response")
    .with_artifacts(vec![artifact])
    .with_metadata(json!({"key": "value"}))
```

## Integration with Existing System

The programmable interface seamlessly integrates with the existing YAML-based system:

### Mixed Agent Types

You can have both YAML-defined and programmatic agents in the same coordinator:

```yaml
# config.yaml - YAML-defined agents
agents:
  - definition:
      name: yaml_agent
      description: "A YAML-defined agent"
      system_prompt: "You are a YAML agent"
      model_settings:
        model: "gpt-4"
```

```rust
// Rust code - programmatic agents
let prog_agent = ProgrammableAgent::builder("prog_agent")
    .description("A programmatic agent")
    .build();

// Register both types
coordinator.register_agent(yaml_agent_def).await?;
coordinator.register_programmable_agent(Box::new(prog_agent)).await?;

// Both appear in agent listings
let (agents, _) = coordinator.list_agents(None).await?;
// agents contains both yaml_agent and prog_agent
```

### A2A Compatibility

Programmable agents are fully compatible with the A2A (Agent-to-Agent) specification:

- Agent definitions are automatically converted to A2A AgentCards
- All A2A endpoints work with programmable agents
- Streaming and event handling supported

## Best Practices

### 1. Error Handling

Always handle errors gracefully:

```rust
async fn invoke(&mut self, task: TaskStep, context: AgentContext, params: Option<serde_json::Value>) -> Result<AgentResponse, AgentError> {
    match some_operation().await {
        Ok(result) => Ok(AgentResponse::text(result)),
        Err(e) => {
            tracing::error!("Agent error: {}", e);
            Err(AgentError::ToolExecution(e.to_string()))
        }
    }
}
```

### 2. Logging

Use structured logging for debugging:

```rust
async fn invoke(&mut self, task: TaskStep, context: AgentContext, params: Option<serde_json::Value>) -> Result<AgentResponse, AgentError> {
    tracing::info!("Agent {} executing task: {}", self.definition().name, task.task);
    
    let result = process_task(&task).await?;
    
    tracing::debug!("Agent {} completed with result length: {}", self.definition().name, result.len());
    
    Ok(AgentResponse::text(result))
}
```

### 3. Resource Management

Be mindful of resource usage in stateful agents:

```rust
struct ResourceAwareAgent {
    definition: AgentDefinition,
    cache: Arc<Mutex<LruCache<String, String>>>,
}

impl Agent for ResourceAwareAgent {
    async fn cleanup(&mut self) -> Result<(), AgentError> {
        // Clean up resources when agent is removed
        self.cache.lock().await.clear();
        Ok(())
    }
}
```

### 4. Testing

Write comprehensive tests for your agents:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_my_agent() {
        let mut agent = MyAgent::new();
        let context = AgentContext::default();
        
        let task = TaskStep {
            task: "test input".to_string(),
            task_images: None,
        };
        
        let result = agent.invoke(task, context, None).await.unwrap();
        assert_eq!(result.content, "expected output");
    }
}
```

## Running Examples

To see the programmable agents in action:

```bash
# Run the examples
cd distri
cargo run --example programmable_agents

# Run tests
cargo test programmable_agents
```

## Migration from YAML

To migrate existing YAML agents to programmable agents:

1. **Identify Custom Logic**: Look for agents that need custom behavior beyond simple LLM calls
2. **Create Agent Struct**: Implement the `Agent` trait with your custom logic
3. **Preserve Configuration**: Keep the same `AgentDefinition` structure for compatibility
4. **Gradual Migration**: You can migrate agents one by one while keeping others as YAML

## Advanced Features

### Custom Tool Integration

```rust
impl Agent for MyAgent {
    async fn get_tools(&self) -> Vec<ServerTools> {
        // Return custom tools this agent provides
        vec![
            ServerTools {
                definition: McpDefinition {
                    name: "custom_tool".to_string(),
                    r#type: McpServerType::Tool,
                    filter: ToolsFilter::All,
                },
                tools: vec![
                    Tool {
                        name: "process_data".to_string(),
                        description: Some("Process data with custom logic".to_string()),
                        input_schema: json!({"type": "object", "properties": {"data": {"type": "string"}}}),
                    }
                ],
            }
        ]
    }
}
```

### Memory Integration

```rust
async fn invoke(&mut self, task: TaskStep, context: AgentContext, _params: Option<serde_json::Value>) -> Result<AgentResponse, AgentError> {
    // Access conversation history
    if let Some(memory_store) = &context.memory_store {
        let messages = memory_store
            .get_messages("agent_id", Some(&context.thread_id))
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;
        
        // Use message history in your logic
        let context_info = format!("Previous messages: {}", messages.len());
        Ok(AgentResponse::text(context_info))
    } else {
        Ok(AgentResponse::text("No memory available".to_string()))
    }
}
```

This programmable interface provides a powerful and flexible way to create sophisticated agents while maintaining full compatibility with the existing distri ecosystem.