# Custom Agents Guide

This guide shows different approaches to create custom agents in Distri without having to redefine all the `BaseAgent` methods manually.

## Problem

When creating custom agents that wrap `StandardAgent`, you typically need to implement all the `BaseAgent` methods by delegating to the inner agent:

```rust
#[async_trait::async_trait]
impl BaseAgent for MyCustomAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Custom("MyCustomAgent".to_string())
    }

    fn get_definition(&self) -> AgentDefinition {
        self.inner.get_definition()
    }

    fn get_description(&self) -> &str {
        self.inner.get_description()
    }

    fn get_tools(&self) -> Vec<&Box<dyn Tool>> {
        self.inner.get_tools()
    }

    fn get_name(&self) -> &str {
        self.inner.get_name()
    }

    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(self.clone())
    }

    fn get_hooks(&self) -> Option<&dyn AgentHooks> {
        Some(self)
    }

    async fn invoke(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: Option<mpsc::Sender<AgentEvent>>,
    ) -> Result<String, AgentError> {
        self.inner.invoke(task, params, context, event_tx).await
    }

    async fn invoke_stream(
        &self,
        task: TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<ExecutorContext>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), AgentError> {
        self.inner.invoke_stream(task, params, context, event_tx).await
    }
}
```

This is a lot of boilerplate code! Here are several solutions:

## Solution 1: Use the `impl_base_agent_delegate!` Macro (Recommended)

The easiest approach is to use the provided macro:

```rust
use crate::agent::{AgentHooks, BaseAgent, StandardAgent};
use crate::memory::TaskStep;
use std::sync::Arc;
use tracing::info;

#[derive(Clone)]
pub struct MyCustomAgent {
    inner: StandardAgent,
    pub custom_data: String,
}

impl std::fmt::Debug for MyCustomAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MyCustomAgent")
            .field("inner", &self.inner)
            .field("custom_data", &self.custom_data)
            .finish()
    }
}

impl MyCustomAgent {
    pub fn new(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        coordinator: Arc<crate::agent::AgentExecutor>,
        context: Arc<crate::agent::ExecutorContext>,
        session_store: Arc<Box<dyn crate::SessionStore>>,
        custom_data: String,
    ) -> Self {
        let inner = StandardAgent::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
        );
        Self { inner, custom_data }
    }
}

// Just one line to implement all BaseAgent methods!
crate::impl_base_agent_delegate!(MyCustomAgent, "MyCustomAgent", inner);

// Only implement the hooks you want to customize
#[async_trait::async_trait]
impl AgentHooks for MyCustomAgent {
    async fn after_task_step(
        &self,
        _task: TaskStep,
        _context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), crate::error::AgentError> {
        info!("🔧 MyCustomAgent: after_task_step called with data: {}", self.custom_data);
        Ok(())
    }
}
```

**Benefits:**
- Minimal boilerplate (just one macro call)
- All `BaseAgent` methods are automatically implemented
- Hooks work correctly with the delegation pattern
- Easy to understand and maintain

## Solution 2: Create a Trait Extension

You can create a trait that provides default implementations:

```rust
use crate::agent::{AgentHooks, BaseAgent, StandardAgent};
use std::sync::Arc;

// Trait that provides default implementations for common BaseAgent methods
pub trait StandardAgentWrapper: AgentHooks {
    fn inner(&self) -> &StandardAgent;
    
    fn agent_type_name(&self) -> &'static str;
    
    // Default implementations
    fn get_definition(&self) -> crate::types::AgentDefinition {
        self.inner().get_definition()
    }
    
    fn get_description(&self) -> &str {
        self.inner().get_description()
    }
    
    fn get_tools(&self) -> Vec<&Box<dyn crate::tools::Tool>> {
        self.inner().get_tools()
    }
    
    fn get_name(&self) -> &str {
        self.inner().get_name()
    }
    
    fn get_hooks(&self) -> Option<&dyn AgentHooks> {
        Some(self)
    }
}

// Implement BaseAgent for any type that implements StandardAgentWrapper
#[async_trait::async_trait]
impl<T: StandardAgentWrapper + Clone + Send + Sync + std::fmt::Debug> BaseAgent for T {
    fn agent_type(&self) -> crate::agent::agent::AgentType {
        crate::agent::agent::AgentType::Custom(self.agent_type_name().to_string())
    }
    
    fn get_definition(&self) -> crate::types::AgentDefinition {
        StandardAgentWrapper::get_definition(self)
    }
    
    fn get_description(&self) -> &str {
        StandardAgentWrapper::get_description(self)
    }
    
    fn get_tools(&self) -> Vec<&Box<dyn crate::tools::Tool>> {
        StandardAgentWrapper::get_tools(self)
    }
    
    fn get_name(&self) -> &str {
        StandardAgentWrapper::get_name(self)
    }
    
    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(self.clone())
    }
    
    fn get_hooks(&self) -> Option<&dyn AgentHooks> {
        StandardAgentWrapper::get_hooks(self)
    }
    
    async fn invoke(
        &self,
        task: crate::memory::TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
        event_tx: Option<tokio::sync::mpsc::Sender<crate::agent::AgentEvent>>,
    ) -> Result<String, crate::error::AgentError> {
        self.inner().invoke(task, params, context, event_tx).await
    }
    
    async fn invoke_stream(
        &self,
        task: crate::memory::TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
        event_tx: tokio::sync::mpsc::Sender<crate::agent::AgentEvent>,
    ) -> Result<(), crate::error::AgentError> {
        self.inner().invoke_stream(task, params, context, event_tx).await
    }
}

// Usage
#[derive(Clone)]
pub struct MyCustomAgent {
    inner: StandardAgent,
    pub custom_data: String,
}

impl StandardAgentWrapper for MyCustomAgent {
    fn inner(&self) -> &StandardAgent {
        &self.inner
    }
    
    fn agent_type_name(&self) -> &'static str {
        "MyCustomAgent"
    }
}

#[async_trait::async_trait]
impl AgentHooks for MyCustomAgent {
    // Implement only the hooks you want to customize
}
```

**Benefits:**
- More flexible than macros
- Can be extended with additional methods
- Type-safe

**Drawbacks:**
- More complex to set up
- Requires understanding of trait bounds

## Solution 3: Composition with a Helper Struct

Create a helper struct that handles the delegation:

```rust
use crate::agent::{AgentHooks, BaseAgent, StandardAgent};
use std::sync::Arc;

// Helper struct that handles BaseAgent delegation
pub struct AgentDelegate {
    inner: StandardAgent,
    agent_type: String,
}

impl AgentDelegate {
    pub fn new(inner: StandardAgent, agent_type: String) -> Self {
        Self { inner, agent_type }
    }
    
    pub fn inner(&self) -> &StandardAgent {
        &self.inner
    }
    
    pub fn inner_mut(&mut self) -> &mut StandardAgent {
        &mut self.inner
    }
}

#[async_trait::async_trait]
impl BaseAgent for AgentDelegate {
    fn agent_type(&self) -> crate::agent::agent::AgentType {
        crate::agent::agent::AgentType::Custom(self.agent_type.clone())
    }
    
    fn get_definition(&self) -> crate::types::AgentDefinition {
        self.inner.get_definition()
    }
    
    fn get_description(&self) -> &str {
        self.inner.get_description()
    }
    
    fn get_tools(&self) -> Vec<&Box<dyn crate::tools::Tool>> {
        self.inner.get_tools()
    }
    
    fn get_name(&self) -> &str {
        self.inner.get_name()
    }
    
    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(self.clone())
    }
    
    fn get_hooks(&self) -> Option<&dyn AgentHooks> {
        None // This would need to be handled by the wrapper
    }
    
    async fn invoke(
        &self,
        task: crate::memory::TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
        event_tx: Option<tokio::sync::mpsc::Sender<crate::agent::AgentEvent>>,
    ) -> Result<String, crate::error::AgentError> {
        self.inner.invoke(task, params, context, event_tx).await
    }
    
    async fn invoke_stream(
        &self,
        task: crate::memory::TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
        event_tx: tokio::sync::mpsc::Sender<crate::agent::AgentEvent>,
    ) -> Result<(), crate::error::AgentError> {
        self.inner.invoke_stream(task, params, context, event_tx).await
    }
}

// Usage
#[derive(Clone)]
pub struct MyCustomAgent {
    delegate: AgentDelegate,
    pub custom_data: String,
}

impl MyCustomAgent {
    pub fn new(
        definition: crate::types::AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        coordinator: Arc<crate::agent::AgentExecutor>,
        context: Arc<crate::agent::ExecutorContext>,
        session_store: Arc<Box<dyn crate::SessionStore>>,
        custom_data: String,
    ) -> Self {
        let inner = StandardAgent::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
        );
        let delegate = AgentDelegate::new(inner, "MyCustomAgent".to_string());
        Self { delegate, custom_data }
    }
}

// Implement BaseAgent by delegating to the helper
#[async_trait::async_trait]
impl BaseAgent for MyCustomAgent {
    fn agent_type(&self) -> crate::agent::agent::AgentType {
        self.delegate.agent_type()
    }
    
    fn get_definition(&self) -> crate::types::AgentDefinition {
        self.delegate.get_definition()
    }
    
    fn get_description(&self) -> &str {
        self.delegate.get_description()
    }
    
    fn get_tools(&self) -> Vec<&Box<dyn crate::tools::Tool>> {
        self.delegate.get_tools()
    }
    
    fn get_name(&self) -> &str {
        self.delegate.get_name()
    }
    
    fn clone_box(&self) -> Box<dyn BaseAgent> {
        Box::new(self.clone())
    }
    
    fn get_hooks(&self) -> Option<&dyn AgentHooks> {
        Some(self)
    }
    
    async fn invoke(
        &self,
        task: crate::memory::TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
        event_tx: Option<tokio::sync::mpsc::Sender<crate::agent::AgentEvent>>,
    ) -> Result<String, crate::error::AgentError> {
        self.delegate.invoke(task, params, context, event_tx).await
    }
    
    async fn invoke_stream(
        &self,
        task: crate::memory::TaskStep,
        params: Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
        event_tx: tokio::sync::mpsc::Sender<crate::agent::AgentEvent>,
    ) -> Result<(), crate::error::AgentError> {
        self.delegate.invoke_stream(task, params, context, event_tx).await
    }
}
```

**Benefits:**
- Reusable helper struct
- Clear separation of concerns

**Drawbacks:**
- Still requires implementing BaseAgent manually
- More complex than the macro approach

## Solution 4: Derive Macro (Advanced)

For the most advanced use case, you could create a derive macro:

```rust
// This would require a procedural macro crate
#[derive(BaseAgentDelegate)]
#[agent_type("MyCustomAgent")]
pub struct MyCustomAgent {
    #[inner]
    inner: StandardAgent,
    pub custom_data: String,
}
```

**Benefits:**
- Most ergonomic syntax
- Compile-time validation

**Drawbacks:**
- Requires procedural macro knowledge
- More complex to implement and maintain

## Recommendation

**Use Solution 1 (the `impl_base_agent_delegate!` macro)** for most cases because:

1. **Simplicity**: Just one line of code to implement all BaseAgent methods
2. **Maintainability**: Easy to understand and modify
3. **Reliability**: The macro is tested and proven to work
4. **Flexibility**: You can still customize any method if needed by implementing it manually

## Example: Updating Existing Agents

Here's how to update the existing `ToolParserAgent`:

**Before:**
```rust
#[async_trait::async_trait]
impl BaseAgent for ToolParserAgent {
    fn agent_type(&self) -> AgentType {
        AgentType::Custom("tool_parser".to_string())
    }
    // ... 50+ lines of boilerplate
}
```

**After:**
```rust
// Just one line!
crate::impl_base_agent_delegate!(ToolParserAgent, "tool_parser", inner);
```

This reduces the code from ~50 lines to just 1 line while maintaining the same functionality!

## Hook Delegation

All these solutions work with the hook delegation pattern we implemented. When you implement `AgentHooks` for your custom agent, the hooks will be called correctly:

```rust
#[async_trait::async_trait]
impl AgentHooks for MyCustomAgent {
    async fn before_llm_step(
        &self,
        messages: &[crate::types::Message],
        params: &Option<serde_json::Value>,
        context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<Vec<crate::types::Message>, crate::error::AgentError> {
        // Your custom logic here
        info!("Custom agent processing {} messages", messages.len());
        Ok(messages.to_vec())
    }
}
```

The hook delegation ensures that your custom hooks are called instead of the default ones, even though the core logic is delegated to the `StandardAgent`.