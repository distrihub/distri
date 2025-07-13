# Custom Agents in Distri

Distri supports custom agent types that allow you to create specialized agents with custom behavior. This guide shows you how to create and register your own custom agents.

## Overview

Custom agents in Distri work through a factory pattern:

1. **Custom Agent**: Implements the `BaseAgent` trait with your custom logic
2. **Agent Factory**: Implements the `AgentFactory` trait to create instances of your custom agent
3. **Registration**: Register your factory with the agent store
4. **Usage**: Use your custom agent like any other agent

## Creating a Custom Agent

Here's a simple example of a custom agent that adds a prefix to all responses:

```rust
use crate::{
    agent::{BaseAgent, ExecutorContext, AgentType},
    memory::TaskStep,
    types::AgentDefinition,
};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct PrefixAgent {
    definition: AgentDefinition,
    prefix: String,
}

impl PrefixAgent {
    pub fn new(definition: AgentDefinition, prefix: String) -> Self {
        Self { definition, prefix }
    }
}

#[async_trait]
impl BaseAgent for PrefixAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Custom("prefix".to_string())
    }

    fn get_definition(&self) -> AgentDefinition {
        self.definition.clone()
    }

    fn get_description(&self) -> &str {
        &self.definition.description
    }

    fn get_tools(&self) -> Vec<&Box<dyn crate::tools::Tool>> {
        vec![] // This agent doesn't use tools
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(Self {
            definition: self.definition.clone(),
            prefix: self.prefix.clone(),
        })
    }

    fn get_name(&self) -> &str {
        &self.definition.name
    }

    async fn invoke(
        &self,
        task: TaskStep,
        _params: Option<serde_json::Value>,
        _context: Arc<ExecutorContext>,
        _event_tx: Option<mpsc::Sender<crate::agent::AgentEvent>>,
    ) -> Result<String, crate::error::AgentError> {
        Ok(format!("{}: {}", self.prefix, task.task))
    }
}
```

## Creating an Agent Factory

Next, create a factory that can instantiate your custom agent:

```rust
use crate::stores::{AgentFactory, SessionStore};

pub struct PrefixAgentFactory {
    prefix: String,
}

impl PrefixAgentFactory {
    pub fn new(prefix: String) -> Self {
        Self { prefix }
    }
}

#[async_trait]
impl AgentFactory for PrefixAgentFactory {
    async fn create_agent(
        &self,
        definition: AgentDefinition,
        _executor: Arc<crate::agent::AgentExecutor>,
        _context: Arc<ExecutorContext>,
        _session_store: Arc<Box<dyn SessionStore>>,
    ) -> anyhow::Result<Box<dyn BaseAgent>> {
        let agent = PrefixAgent::new(definition, self.prefix.clone());
        Ok(Box::new(agent))
    }

    fn agent_type(&self) -> &str {
        "prefix"
    }
}
```

## Registering and Using Your Custom Agent

Here's how to register and use your custom agent:

```rust
use crate::{
    agent::AgentExecutor,
    stores::AgentStore,
    types::{AgentDefinition, ModelSettings},
};

async fn use_custom_agent() -> anyhow::Result<()> {
    // Create an executor
    let executor = AgentExecutor::initialize(&config).await?;
    
    // Register your custom agent factory
    let prefix_factory = Box::new(PrefixAgentFactory::new("CUSTOM".to_string()));
    executor.agent_store.register_factory(prefix_factory).await?;

    // Create an agent definition
    let agent_def = AgentDefinition {
        name: "my_prefix_agent".to_string(),
        description: "A custom agent that adds a prefix".to_string(),
        model_settings: ModelSettings::default(),
        mcp_servers: vec![],
        system_prompt: "You are a helpful assistant.".to_string(),
        plan_config: None,
    };

    // Register the agent with the custom type
    let agent = executor.register_custom_agent(agent_def, "prefix").await?;

    // Use the agent
    let task = TaskStep {
        task: "Hello world".to_string(),
        task_id: "test_task".to_string(),
        context_id: "test_context".to_string(),
    };

    let context = Arc::new(ExecutorContext::default());
    let result = agent.invoke(task, None, context, None).await?;
    println!("Result: {}", result); // Output: "CUSTOM: Hello world"

    Ok(())
}
```

## Key Points

1. **Agent Type**: Use `AgentType::Custom("your_type_name".to_string())` to identify your custom agent
2. **Factory Pattern**: The factory pattern allows the agent store to create instances of your custom agent when needed
3. **Minimal Changes**: This approach requires minimal changes to the existing codebase
4. **Flexibility**: You can create any type of custom agent with specialized behavior

## Advanced Custom Agents

For more complex custom agents, you can:

- Implement streaming responses with `invoke_stream`
- Use tools and MCP servers
- Add custom validation logic
- Implement custom memory management
- Add custom event handling

## Example: Character Count Agent

Here's another example of a custom agent that counts characters:

```rust
pub struct CharCountAgent {
    definition: AgentDefinition,
}

#[async_trait]
impl BaseAgent for CharCountAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Custom("char_count".to_string())
    }

    // ... other trait methods ...

    async fn invoke(
        &self,
        task: TaskStep,
        _params: Option<serde_json::Value>,
        _context: Arc<ExecutorContext>,
        _event_tx: Option<mpsc::Sender<crate::agent::AgentEvent>>,
    ) -> Result<String, crate::error::AgentError> {
        Ok(format!("Task has {} characters", task.task.len()))
    }
}
```

This approach gives you complete flexibility to create any type of custom agent while maintaining compatibility with the existing Distri infrastructure.