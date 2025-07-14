# Custom Agents Sample

This sample demonstrates how to create and use custom agents with the Distri agent factory system.

## Overview

The custom agents sample shows how to:
1. Create custom agent implementations that extend `StandardAgent`
2. Register custom agent factories with the executor
3. Use custom agents alongside standard agents

## Custom Agents

### LoggingAgent

A custom agent that adds comprehensive logging to all agent operations. It logs:
- Task start
- LLM calls
- Tool executions
- Tool responses
- Task completion

### FilteringAgent

A custom agent that filters content based on banned words. It replaces banned words with asterisks in the final output.

## Usage

### 1. Register Custom Agent Factories

```rust
use custom_agents::{create_logging_agent_factory, create_filtering_agent_factory};

// Register logging agent factory
executor.register_agent_factory("logging".to_string(), create_logging_agent_factory()).await;

// Register filtering agent factory with banned words
let banned_words = vec!["badword".to_string(), "inappropriate".to_string()];
executor.register_agent_factory("filtering".to_string(), create_filtering_agent_factory(banned_words)).await;
```

### 2. Use Custom Agents

Once registered, custom agents can be used just like standard agents:

```rust
// The executor will automatically use the appropriate factory
// based on the agent definition or default to standard
let agent = executor.create_agent_from_definition(definition).await?;
let result = agent.invoke(task, params, context, event_tx).await?;
```

## Configuration

Custom agents can be configured in YAML just like standard agents:

```yaml
agents:
  - name: "logging-assistant"
    description: "A helpful assistant with comprehensive logging"
    system_prompt: "You are a helpful assistant."
    model_settings:
      model: "gpt-4o-mini"
      temperature: 0.7
      max_tokens: 1000
    # The agent type could be specified here in the future
    # agent_type: "logging"
```

## Extending

To create your own custom agent:

1. Implement the `BaseAgent` trait
2. Optionally implement `AgentHooks` for custom behavior
3. Create a factory function that returns your agent
4. Register the factory with the executor

Example:

```rust
pub struct MyCustomAgent {
    inner: StandardAgent,
    // Your custom fields
}

impl BaseAgent for MyCustomAgent {
    // Implement required methods
}

pub fn create_my_custom_agent_factory() -> Arc<AgentFactoryFn> {
    Arc::new(|definition, tools_registry, executor, context, session_store| {
        Box::new(MyCustomAgent::new(
            definition,
            tools_registry,
            executor,
            context,
            session_store,
        ))
    })
}
```

## Benefits

- **Separation of Concerns**: Agent definitions are stored separately from agent instances
- **Factory Pattern**: Easy to register and use custom agent types
- **Backward Compatibility**: Standard agents continue to work unchanged
- **Extensibility**: Easy to add new agent types without modifying core code