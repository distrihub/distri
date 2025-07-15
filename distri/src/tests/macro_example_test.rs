use crate::agent::{AgentHooks, BaseAgent, StandardAgent};
use crate::memory::TaskStep;
use crate::types::{AgentDefinition, ModelSettings};
use std::sync::Arc;
use tracing::info;

// Example 1: Using the impl_base_agent_delegate macro
#[derive(Clone)]
struct SimpleCustomAgent {
    inner: StandardAgent,
    custom_field: String,
}

impl std::fmt::Debug for SimpleCustomAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleCustomAgent")
            .field("inner", &self.inner)
            .field("custom_field", &self.custom_field)
            .finish()
    }
}

impl SimpleCustomAgent {
    pub fn new(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        coordinator: Arc<crate::agent::AgentExecutor>,
        context: Arc<crate::agent::ExecutorContext>,
        session_store: Arc<Box<dyn crate::SessionStore>>,
        custom_field: String,
    ) -> Self {
        let inner = StandardAgent::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
        );
        Self {
            inner,
            custom_field,
        }
    }
}

// Use the macro to automatically implement BaseAgent
crate::impl_base_agent_delegate!(SimpleCustomAgent, "SimpleCustomAgent", inner);

#[async_trait::async_trait]
impl AgentHooks for SimpleCustomAgent {
    async fn after_task_step(
        &self,
        _task: TaskStep,
        _context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), crate::error::AgentError> {
        info!("🔧 SimpleCustomAgent: after_task_step called with custom_field: {}", self.custom_field);
        Ok(())
    }
}

// Example 2: Using the custom_agent macro (more advanced)
#[derive(Clone)]
pub struct AdvancedCustomAgent {
    inner: StandardAgent,
    pub counter: Arc<tokio::sync::Mutex<i32>>,
    pub custom_config: String,
}

impl std::fmt::Debug for AdvancedCustomAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdvancedCustomAgent")
            .field("inner", &self.inner)
            .field("counter", &"Arc<Mutex<i32>>")
            .field("custom_config", &self.custom_config)
            .finish()
    }
}

// Use the macro to automatically implement BaseAgent
crate::impl_base_agent_delegate!(AdvancedCustomAgent, "AdvancedCustomAgent", inner);

impl AdvancedCustomAgent {
    pub fn new_with_config(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        coordinator: Arc<crate::agent::AgentExecutor>,
        context: Arc<crate::agent::ExecutorContext>,
        session_store: Arc<Box<dyn crate::SessionStore>>,
        custom_config: String,
    ) -> Self {
        let inner = StandardAgent::new(
            definition,
            tools_registry,
            coordinator,
            context,
            session_store,
        );
        Self {
            inner,
            counter: Arc::new(tokio::sync::Mutex::new(0)),
            custom_config,
        }
    }

    pub async fn increment_counter(&self) -> i32 {
        let mut counter = self.counter.lock().await;
        *counter += 1;
        *counter
    }
}

#[async_trait::async_trait]
impl AgentHooks for AdvancedCustomAgent {
    async fn before_llm_step(
        &self,
        _messages: &[crate::types::Message],
        _params: &Option<serde_json::Value>,
        _context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<Vec<crate::types::Message>, crate::error::AgentError> {
        let counter = self.increment_counter().await;
        info!("🔧 AdvancedCustomAgent: before_llm_step called (counter: {}, config: {})", 
              counter, self.custom_config);
        Ok(vec![])
    }
}

#[tokio::test]
async fn test_macro_based_custom_agents() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Test SimpleCustomAgent
    let stores = crate::types::StoreConfig::default().initialize().await?;
    let executor = crate::agent::AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()?;
    let executor = Arc::new(executor);

    let agent_def = AgentDefinition {
        name: "test-simple-agent".to_string(),
        description: "A test simple custom agent".to_string(),
        agent_type: Some("simple_custom".to_string()),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(1),
        ..Default::default()
    };

    let simple_agent = SimpleCustomAgent::new(
        agent_def.clone(),
        Arc::new(crate::tools::LlmToolsRegistry::default()),
        executor.clone(),
        Arc::new(crate::agent::ExecutorContext::default()),
        Arc::new(Box::new(crate::stores::noop::NoopSessionStore::default()) as Box<dyn crate::SessionStore>),
        "test_value".to_string(),
    );

    // Verify the macro worked correctly
    assert_eq!(simple_agent.get_name(), "test-simple-agent");
    assert_eq!(simple_agent.agent_type(), crate::agent::agent::AgentType::Custom("SimpleCustomAgent".to_string()));
    assert!(simple_agent.get_hooks().is_some());
    assert_eq!(simple_agent.custom_field, "test_value");

    // Test AdvancedCustomAgent
    let advanced_agent = AdvancedCustomAgent::new_with_config(
        agent_def,
        Arc::new(crate::tools::LlmToolsRegistry::default()),
        executor,
        Arc::new(crate::agent::ExecutorContext::default()),
        Arc::new(Box::new(crate::stores::noop::NoopSessionStore::default()) as Box<dyn crate::SessionStore>),
        "advanced_config".to_string(),
    );

    // Verify the macro worked correctly
    assert_eq!(advanced_agent.get_name(), "test-simple-agent");
    assert_eq!(advanced_agent.agent_type(), crate::agent::agent::AgentType::Custom("AdvancedCustomAgent".to_string()));
    assert!(advanced_agent.get_hooks().is_some());
    assert_eq!(advanced_agent.custom_config, "advanced_config");

    // Test counter functionality
    let initial_counter = advanced_agent.increment_counter().await;
    assert_eq!(initial_counter, 1);

    info!("✅ Macro-based custom agents test completed successfully");
    Ok(())
}