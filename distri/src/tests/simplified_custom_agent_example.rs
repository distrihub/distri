use crate::agent::{AgentHooks, BaseAgent, StandardAgent};
use crate::memory::TaskStep;
use crate::types::{AgentDefinition, ModelSettings};
use std::sync::Arc;
use tracing::info;

// Example: Creating a custom agent with minimal boilerplate
#[derive(Clone)]
pub struct MinimalCustomAgent {
    inner: StandardAgent,
    pub custom_data: String,
}

impl std::fmt::Debug for MinimalCustomAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MinimalCustomAgent")
            .field("inner", &self.inner)
            .field("custom_data", &self.custom_data)
            .finish()
    }
}

impl MinimalCustomAgent {
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
crate::impl_base_agent_delegate!(MinimalCustomAgent, "MinimalCustomAgent", inner);

// Only implement the hooks you want to customize
#[async_trait::async_trait]
impl AgentHooks for MinimalCustomAgent {
    async fn after_task_step(
        &self,
        _task: TaskStep,
        _context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), crate::error::AgentError> {
        info!("🔧 MinimalCustomAgent: after_task_step called with data: {}", self.custom_data);
        Ok(())
    }
}

#[tokio::test]
async fn test_simplified_custom_agent() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let stores = crate::types::StoreConfig::default().initialize().await?;
    let executor = crate::agent::AgentExecutorBuilder::default()
        .with_stores(stores)
        .build()?;
    let executor = Arc::new(executor);

    let agent_def = AgentDefinition {
        name: "test-minimal-agent".to_string(),
        description: "A test minimal custom agent".to_string(),
        agent_type: Some("minimal_custom".to_string()),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(1),
        ..Default::default()
    };

    let agent = MinimalCustomAgent::new(
        agent_def,
        Arc::new(crate::tools::LlmToolsRegistry::default()),
        executor,
        Arc::new(crate::agent::ExecutorContext::default()),
        Arc::new(Box::new(crate::stores::noop::NoopSessionStore::default()) as Box<dyn crate::SessionStore>),
        "my_custom_data".to_string(),
    );

    // Verify the macro worked correctly
    assert_eq!(agent.get_name(), "test-minimal-agent");
    assert_eq!(agent.agent_type(), crate::agent::agent::AgentType::Custom("MinimalCustomAgent".to_string()));
    assert!(agent.get_hooks().is_some());
    assert_eq!(agent.custom_data, "my_custom_data");

    info!("✅ Simplified custom agent test completed successfully");
    Ok(())
}