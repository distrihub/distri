# Step-Based Agent Refactoring Summary

## Overview

The agent system has been successfully refactored to implement a **step-based execution model** where the `step` function represents one iteration inside the execution loop, rather than the entire execution. This provides much more granular control over the agent execution process.

## Key Architecture Changes

### 1. CustomAgent Trait - Step-Based Interface

The `CustomAgent` trait now provides one step in the execution loop:

```rust
#[async_trait::async_trait]
pub trait CustomAgent: Send + Sync + std::fmt::Debug {
    /// Execute one step in the agent execution loop
    /// This is called for each iteration and should implement the agent's custom logic
    async fn step(
        &self,
        messages: &[Message],
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Result<StepResult, AgentError>;

    /// Clone the custom agent (required for object safety)
    fn clone_box(&self) -> Box<dyn CustomAgent>;
}
```

### 2. StepResult - Flexible Step Outcomes

```rust
#[derive(Debug)]
pub enum StepResult {
    /// Continue with more iterations, with new messages to add
    Continue(Vec<Message>),
    /// Finish execution with this final response
    Finish(String),
    /// Handle tool calls (for custom agents that want to manage tools)
    ToolCalls(Vec<crate::types::ToolCall>),
}
```

### 3. Agent Implementation - Dual Execution Paths

The `Agent` struct now implements different step behaviors:

- **For `AgentRecord::Local`**: Uses `local_step()` which performs standard LLM calls with tool handling
- **For `AgentRecord::Runnable`**: Delegates to `CustomAgent::step()` for custom logic

```rust
impl Agent {
    /// Execute one step in the execution loop
    /// For Local agents: executes LLM call with tool handling
    /// For Runnable agents: calls CustomAgent::step
    async fn step(
        &self,
        messages: &[Message],
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
    ) -> Result<StepResult, AgentError> {
        if let Some(custom_agent) = &self.custom_agent {
            // For runnable agents, delegate to CustomAgent::step
            custom_agent.step(messages, params, context, self.session_store.clone()).await
        } else {
            // For local agents, execute standard LLM call with tool handling
            self.local_step(messages, params, context).await
        }
    }
}
```

## Execution Flow

### Local Agents (YAML-based)
1. **Setup**: System prompt, task, planning (if enabled)
2. **Loop**: For each iteration:
   - Call `local_step()` which executes LLM call
   - Handle tool calls automatically
   - Continue or finish based on LLM response
3. **Finish**: Store final result and return

### Custom Agents (Runnable)
1. **Setup**: System prompt, task, planning (if enabled)  
2. **Loop**: For each iteration:
   - Call `CustomAgent::step()` with current state
   - Custom agent implements its own logic
   - Return `StepResult` to control flow
3. **Finish**: Store final result and return

## Example Custom Agent

Here's a simple example of a custom agent using the new step-based approach:

```rust
use crate::{
    agent::{CustomAgent, StepResult},
    error::AgentError,
    executor::LLMExecutor,
    types::{AgentDefinition, ModelSettings, Message, MessageContent, MessageRole},
    memory::{MemoryStep, SystemStep},
    SessionStore,
};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ExampleCustomAgent {
    pub name: String,
    pub step_count: std::sync::Arc<std::sync::Mutex<i32>>,
}

impl ExampleCustomAgent {
    pub fn new(name: String) -> Self {
        Self {
            name,
            step_count: std::sync::Arc::new(std::sync::Mutex::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl CustomAgent for ExampleCustomAgent {
    async fn step(
        &self,
        messages: &[Message],
        params: Option<serde_json::Value>,
        context: Arc<CoordinatorContext>,
        session_store: Arc<Box<dyn SessionStore>>,
    ) -> Result<StepResult, AgentError> {
        // Increment step counter
        let step_num = {
            let mut count = self.step_count.lock().unwrap();
            *count += 1;
            *count
        };

        // Log the step
        tracing::info!("[{}] Executing step {}", self.name, step_num);

        // Write custom preprocessing to session
        let custom_message = MemoryStep::System(SystemStep {
            system_prompt: format!("Custom step {} by {}", step_num, self.name),
        });
        session_store
            .store_step(&context.thread_id, custom_message)
            .await
            .map_err(|e| AgentError::Session(e.to_string()))?;

        // Example: Finish after 2 steps
        if step_num >= 2 {
            return Ok(StepResult::Finish(format!(
                "Custom agent {} completed after {} steps",
                self.name, step_num
            )));
        }

        // Otherwise, call LLM and continue
        let executor = LLMExecutor::new(
            AgentDefinition {
                name: self.name.clone(),
                model_settings: ModelSettings::default(),
                ..Default::default()
            },
            vec![],
            context.clone(),
            None,
            Some(format!("{}:step{}", self.name, step_num)),
        );

        let response = executor.execute(messages, params).await?;
        let content = LLMExecutor::extract_first_choice(&response);

        // Add response message and continue
        let response_message = Message {
            role: MessageRole::Assistant,
            name: Some(self.name.clone()),
            content: vec![MessageContent {
                content_type: "text".to_string(),
                text: Some(format!("Step {}: {}", step_num, content)),
                image: None,
            }],
            tool_calls: vec![],
        };

        Ok(StepResult::Continue(vec![response_message]))
    }

    fn clone_box(&self) -> Box<dyn CustomAgent> {
        Box::new(self.clone())
    }
}
```

## Usage Example

```rust
// Create custom agent
let custom_agent = ExampleCustomAgent::new("example".to_string());

// Create agent definition
let agent_def = AgentDefinition {
    name: "example_agent".to_string(),
    description: "Example custom agent".to_string(),
    model_settings: ModelSettings::default(),
    ..Default::default()
};

// Register as runnable agent
let runnable_agent = AgentRecord::Runnable(agent_def, Box::new(custom_agent));
coordinator.register_agent(runnable_agent).await?;

// Execute (same interface as local agents)
let result = coordinator.execute("example_agent", task, params, context).await?;
```

## Key Benefits

1. **Granular Control**: Each step can be customized individually
2. **Flexible Flow Control**: Custom agents can decide when to continue or finish
3. **State Management**: Each step has access to current messages and context
4. **Tool Integration**: Custom agents can handle tools or delegate to the framework
5. **Backward Compatibility**: Local (YAML) agents work unchanged
6. **Debugging**: Easy to debug individual steps and state transitions

## Comparison: Before vs After

### Before (Previous Architecture)
- CustomAgent implemented entire execution
- No granular control over individual steps
- Harder to manage complex flows
- Limited integration with framework features

### After (Step-Based Architecture)
- CustomAgent implements one step at a time
- Full control over each iteration
- Easy to build complex multi-step behaviors
- Seamless integration with framework infrastructure

## Advanced Use Cases

The step-based approach enables sophisticated patterns:

1. **Multi-Stage Processing**: Different logic for different steps
2. **Conditional Flows**: Different paths based on step outcomes
3. **State Machines**: Implement complex state transitions
4. **Iterative Refinement**: Gradually improve responses over steps
5. **Tool Orchestration**: Custom tool calling strategies

This architecture provides a solid foundation for building sophisticated, programmable agents while maintaining simplicity for basic use cases.