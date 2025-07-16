use crate::{
    agent::{AgentExecutorBuilder, AgentHooks, AgentType, BaseAgent, StandardAgent},
    delegate_base_agent,
    memory::TaskStep,
    types::{AgentDefinition, ModelSettings},
};
use anyhow::Result;
use std::sync::Arc;
use tracing::info;

#[derive(Clone)]
struct TestHookAgent {
    inner: StandardAgent,
    hook_calls: Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl std::fmt::Debug for TestHookAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestHookAgent").finish()
    }
}

impl TestHookAgent {
    fn new(
        definition: AgentDefinition,
        tools_registry: Arc<crate::tools::LlmToolsRegistry>,
        executor: Arc<crate::agent::AgentExecutor>,
        session_store: Arc<Box<dyn crate::SessionStore>>,
    ) -> Self {
        let inner = StandardAgent::new(definition, tools_registry, executor, session_store);
        Self {
            inner,
            hook_calls: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }
}

delegate_base_agent!(TestHookAgent, "TestHookAgent", inner);
#[async_trait::async_trait]
impl AgentHooks for TestHookAgent {
    async fn after_task_step(
        &self,
        _task: TaskStep,
        _context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<(), crate::error::AgentError> {
        let mut calls = self.hook_calls.lock().await;
        calls.push("after_task_step".to_string());
        info!("🔧 TestHookAgent: after_task_step called");
        Ok(())
    }

    async fn before_llm_step(
        &self,
        _messages: &[crate::types::Message],
        _params: &Option<serde_json::Value>,
        _context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<Vec<crate::types::Message>, crate::error::AgentError> {
        let mut calls = self.hook_calls.lock().await;
        calls.push("before_llm_step".to_string());
        info!("🔧 TestHookAgent: before_llm_step called");
        Ok(vec![])
    }

    async fn after_finish(
        &self,
        step_result: crate::agent::StepResult,
        _context: Arc<crate::agent::ExecutorContext>,
    ) -> Result<crate::agent::StepResult, crate::error::AgentError> {
        let mut calls = self.hook_calls.lock().await;
        calls.push("after_finish".to_string());
        info!("🔧 TestHookAgent: after_finish called");
        Ok(step_result)
    }
}

#[tokio::test]
async fn test_hook_delegation() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Create a custom agent factory for testing
    let custom_factory = Arc::new(|definition, tools_registry, executor, session_store| {
        Box::new(TestHookAgent::new(
            definition,
            tools_registry,
            executor,
            session_store,
        )) as Box<dyn BaseAgent>
    });

    // Create executor with custom factory
    let stores = crate::types::StoreConfig::default().initialize().await?;
    let mut factory_registry = crate::agent::factory::AgentFactoryRegistry::new();
    factory_registry.register_factory("test_hook".to_string(), custom_factory);

    let executor = AgentExecutorBuilder::default()
        .with_stores(stores)
        .with_agent_factory(Arc::new(tokio::sync::RwLock::new(factory_registry)))
        .build()?;

    let executor = Arc::new(executor);

    // Create agent definition
    let agent_def = AgentDefinition {
        name: "test-hook-agent".to_string(),
        description: "A test agent for hook delegation".to_string(),
        agent_type: Some("test_hook".to_string()),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        mcp_servers: vec![],
        model_settings: ModelSettings::default(),
        history_size: Some(10),
        max_iterations: Some(1), // Limit iterations to avoid API calls
        ..Default::default()
    };

    // Register agent definition
    executor
        .register_agent_definition(agent_def.clone())
        .await?;

    // Test creating agent from definition
    let agent = executor.create_agent_from_definition(agent_def).await?;
    assert_eq!(agent.get_name(), "test-hook-agent");
    assert_eq!(
        agent.agent_type(),
        AgentType::Custom("TestHookAgent".to_string())
    );

    // Verify that the agent has hooks
    assert!(agent.get_hooks().is_some());

    info!("✅ Hook delegation test completed successfully");
    Ok(())
}
