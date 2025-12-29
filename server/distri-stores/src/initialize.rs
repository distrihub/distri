use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::InMemoryExternalToolCallsStore;
use crate::diesel_store::DieselStoreBuilder;
#[cfg(all(not(feature = "sqlite"), feature = "postgres"))]
use crate::diesel_store::PgStoreBuilder;
#[cfg(feature = "sqlite")]
use crate::diesel_store::SqliteStoreBuilder;
use crate::workflow::InMemoryWorkflowStore;
use anyhow::{Result, anyhow};
use distri_types::configuration::{DbConnectionConfig, StoreType};
pub use distri_types::stores::*;
use distri_types::workflow::WorkflowStore;
use distri_types::{ToolAuthStore, configuration::StoreConfig};

type StoreInitializerFuture = Pin<Box<dyn Future<Output = Result<Arc<dyn StoreFactory>>> + Send>>;

pub type StoreInitializer =
    Arc<dyn Fn(Option<DbConnectionConfig>) -> StoreInitializerFuture + Send + Sync>;

pub trait StoreFactory: Send + Sync {
    fn agent_store(&self) -> Arc<dyn AgentStore>;
    fn tool_auth_store(&self) -> Arc<dyn ToolAuthStore>;
    fn memory_store(&self) -> Arc<dyn MemoryStore>;
    fn thread_store(&self) -> Arc<dyn ThreadStore>;
    fn task_store(&self) -> Arc<dyn TaskStore>;
    fn scratchpad_store(&self) -> Arc<dyn ScratchpadStore>;
    fn session_store(&self) -> Arc<dyn SessionStore>;
    fn plugin_catalog_store(&self) -> Arc<dyn PluginCatalogStore>;
    fn browser_session_store(&self) -> Arc<dyn BrowserSessionStore>;
    fn settings_store(&self) -> Arc<dyn SettingsStore>;
}

impl<Conn> StoreFactory for DieselStoreBuilder<Conn>
where
    Conn: crate::diesel_store::DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>:
        diesel_async::methods::ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery:
        diesel::query_builder::QueryFragment<<Conn as diesel_async::AsyncConnectionCore>::Backend>,
    <Conn as diesel_async::AsyncConnectionCore>::Backend:
        diesel::backend::DieselReserveSpecialization,
{
    fn agent_store(&self) -> Arc<dyn AgentStore> {
        Arc::new(DieselStoreBuilder::agent_store(self)) as Arc<dyn AgentStore>
    }

    fn tool_auth_store(&self) -> Arc<dyn ToolAuthStore> {
        Arc::new(DieselStoreBuilder::tool_auth_store(self)) as Arc<dyn ToolAuthStore>
    }

    fn memory_store(&self) -> Arc<dyn MemoryStore> {
        Arc::new(DieselStoreBuilder::memory_store(self)) as Arc<dyn MemoryStore>
    }

    fn thread_store(&self) -> Arc<dyn ThreadStore> {
        Arc::new(DieselStoreBuilder::thread_store(self)) as Arc<dyn ThreadStore>
    }

    fn task_store(&self) -> Arc<dyn TaskStore> {
        Arc::new(DieselStoreBuilder::task_store(self)) as Arc<dyn TaskStore>
    }

    fn scratchpad_store(&self) -> Arc<dyn ScratchpadStore> {
        Arc::new(DieselStoreBuilder::scratchpad_store(self)) as Arc<dyn ScratchpadStore>
    }

    fn session_store(&self) -> Arc<dyn SessionStore> {
        Arc::new(DieselStoreBuilder::session_store(self)) as Arc<dyn SessionStore>
    }

    fn plugin_catalog_store(&self) -> Arc<dyn PluginCatalogStore> {
        Arc::new(DieselStoreBuilder::plugin_catalog_store(self)) as Arc<dyn PluginCatalogStore>
    }

    fn browser_session_store(&self) -> Arc<dyn BrowserSessionStore> {
        Arc::new(DieselStoreBuilder::browser_session_store(self)) as Arc<dyn BrowserSessionStore>
    }

    fn settings_store(&self) -> Arc<dyn SettingsStore> {
        Arc::new(DieselStoreBuilder::settings_store(self)) as Arc<dyn SettingsStore>
    }
}

fn boxed_initializer<F, Fut, Factory>(initializer: F) -> StoreInitializer
where
    F: Fn(Option<DbConnectionConfig>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Factory>> + Send + 'static,
    Factory: StoreFactory + 'static,
{
    Arc::new(move |config| {
        let fut = initializer(config);
        Box::pin(async move {
            let factory = fut.await?;
            Ok(Arc::new(factory) as Arc<dyn StoreFactory>)
        }) as StoreInitializerFuture
    })
}

/// Builder for InitializedStores that allows optional pre-initialized stores
/// Stores are only initialized if they're not already provided
pub struct StoreBuilder {
    config: StoreConfig,
    store_initializers: HashMap<StoreType, StoreInitializer>,

    // Optional pre-initialized stores
    pub agent_store: Option<Arc<dyn AgentStore>>,
    pub tool_auth_store: Option<Arc<dyn ToolAuthStore>>,
    pub memory_store: Option<Option<Arc<dyn MemoryStore>>>,
    pub thread_store: Option<Arc<dyn ThreadStore>>,
    pub task_store: Option<Arc<dyn TaskStore>>,
    pub scratchpad_store: Option<Arc<dyn ScratchpadStore>>,
    pub session_store: Option<Arc<dyn SessionStore>>,
    pub workflow_store: Option<Arc<dyn WorkflowStore>>,
    pub external_tool_calls_store: Option<Arc<dyn ExternalToolCallsStore>>,
    pub plugin_store: Option<Arc<dyn PluginCatalogStore>>,
    pub browser_session_store: Option<Arc<dyn BrowserSessionStore>>,
    pub settings_store: Option<Arc<dyn SettingsStore>>,
}

impl StoreBuilder {
    pub fn new(config: StoreConfig) -> Self {
        Self {
            config,
            store_initializers: HashMap::new(),
            agent_store: None,
            tool_auth_store: None,
            memory_store: None,
            thread_store: None,
            task_store: None,
            scratchpad_store: None,
            session_store: None,
            workflow_store: None,
            external_tool_calls_store: None,
            plugin_store: None,
            browser_session_store: None,
            settings_store: None,
        }
        .register_default_store_types()
    }

    pub fn register_store_types<F, Fut, Factory>(
        mut self,
        store_type: StoreType,
        initializer: F,
    ) -> Self
    where
        F: Fn(Option<DbConnectionConfig>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Factory>> + Send + 'static,
        Factory: StoreFactory + 'static,
    {
        self.store_initializers
            .insert(store_type, boxed_initializer(initializer));
        self
    }

    fn register_default_store_types(mut self) -> Self {
        #[cfg(feature = "sqlite")]
        {
            let sqlite_initializer = boxed_initializer(initialize_sqlite);
            self.store_initializers
                .insert(StoreType::Sqlite, sqlite_initializer);
        }
        #[cfg(all(not(feature = "sqlite"), feature = "postgres"))]
        {
            let postgres_initializer = boxed_initializer(initialize_postgres);
            self.store_initializers
                .insert(StoreType::Postgres, postgres_initializer);
        }
        self
    }

    async fn resolve_factory(
        &self,
        store_type: &StoreType,
        db_config: Option<DbConnectionConfig>,
    ) -> Result<Arc<dyn StoreFactory>> {
        let initializer = self
            .store_initializers
            .get(store_type)
            .ok_or_else(|| anyhow!("store type {} is not registered", store_type.label()))?;
        initializer(db_config).await
    }

    /// Set a pre-initialized agent store (won't be reinitialized)
    pub fn with_agent_store(mut self, store: Arc<dyn AgentStore>) -> Self {
        self.agent_store = Some(store);
        self
    }

    /// Set a pre-initialized tool auth store (won't be reinitialized)
    pub fn with_tool_auth_store(mut self, store: Arc<dyn ToolAuthStore>) -> Self {
        self.tool_auth_store = Some(store);
        self
    }

    /// Set a pre-initialized memory store (won't be reinitialized)
    pub fn with_memory_store(mut self, store: Option<Arc<dyn MemoryStore>>) -> Self {
        self.memory_store = Some(store);
        self
    }

    /// Set pre-initialized session stores (won't be reinitialized)
    pub fn with_session_stores(
        mut self,
        thread_store: Arc<dyn ThreadStore>,
        task_store: Arc<dyn TaskStore>,
        scratchpad_store: Arc<dyn ScratchpadStore>,
        session_store: Arc<dyn SessionStore>,
    ) -> Self {
        self.thread_store = Some(thread_store);
        self.task_store = Some(task_store);
        self.scratchpad_store = Some(scratchpad_store);
        self.session_store = Some(session_store);
        self
    }

    /// Set a pre-initialized workflow store (won't be reinitialized)
    pub fn with_workflow_store(mut self, store: Arc<dyn WorkflowStore>) -> Self {
        self.workflow_store = Some(store);
        self
    }

    /// Set a pre-initialized external tool calls store (won't be reinitialized)
    pub fn with_external_tool_calls_store(
        mut self,
        store: Arc<dyn ExternalToolCallsStore>,
    ) -> Self {
        self.external_tool_calls_store = Some(store);
        self
    }

    /// Set a pre-initialized plugin catalog store (won't be reinitialized)
    pub fn with_plugin_store(mut self, store: Arc<dyn PluginCatalogStore>) -> Self {
        self.plugin_store = Some(store);
        self
    }

    /// Set a pre-initialized browser session store (won't be reinitialized)
    pub fn with_browser_session_store(mut self, store: Arc<dyn BrowserSessionStore>) -> Self {
        self.browser_session_store = Some(store);
        self
    }

    /// Set a pre-initialized settings store (won't be reinitialized)
    pub fn with_settings_store(mut self, store: Arc<dyn SettingsStore>) -> Self {
        self.settings_store = Some(store);
        self
    }

    /// Build InitializedStores, initializing only stores that weren't pre-provided
    pub async fn build(self) -> Result<InitializedStores> {
        let metadata_factory = self
            .resolve_factory(
                &self.config.metadata.store_type,
                self.config.metadata.db_config.clone(),
            )
            .await?;

        let agent_store = self
            .agent_store
            .clone()
            .unwrap_or_else(|| metadata_factory.agent_store());
        let tool_auth_store = self
            .tool_auth_store
            .clone()
            .unwrap_or_else(|| metadata_factory.tool_auth_store());
        let plugin_store = self
            .plugin_store
            .clone()
            .unwrap_or_else(|| metadata_factory.plugin_catalog_store());
        let browser_session_store = self
            .browser_session_store
            .clone()
            .unwrap_or_else(|| metadata_factory.browser_session_store());
        let settings_store = self
            .settings_store
            .clone()
            .unwrap_or_else(|| metadata_factory.settings_store());

        // Initialize memory store if configured and not provided
        let memory_store = if let Some(store) = self.memory_store.clone() {
            store
        } else if let Some(memory_config) = &self.config.memory {
            let factory = self
                .resolve_factory(&memory_config.store_type, memory_config.db_config.clone())
                .await?;
            Some(factory.memory_store())
        } else {
            None
        };

        // Initialize session stores if not provided
        let (thread_store, task_store, scratchpad_store, session_store) =
            if self.thread_store.is_some()
                && self.task_store.is_some()
                && self.scratchpad_store.is_some()
                && self.session_store.is_some()
            {
                // All provided, use them
                (
                    self.thread_store.unwrap(),
                    self.task_store.unwrap(),
                    self.scratchpad_store.unwrap(),
                    self.session_store.unwrap(),
                )
            } else if self.config.session.ephemeral {
                #[cfg(not(feature = "sqlite"))]
                {
                    return Err(anyhow!(
                        "ephemeral session stores require the sqlite feature to be enabled"
                    ));
                }
                #[cfg(feature = "sqlite")]
                {
                    let placeholder_factory =
                        Arc::new(initialize_ephemeral_sqlite().await?) as Arc<dyn StoreFactory>;
                    (
                        self.thread_store
                            .unwrap_or_else(|| placeholder_factory.thread_store()),
                        self.task_store
                            .unwrap_or_else(|| placeholder_factory.task_store()),
                        self.scratchpad_store
                            .unwrap_or_else(|| placeholder_factory.scratchpad_store()),
                        self.session_store
                            .unwrap_or_else(|| placeholder_factory.session_store()),
                    )
                }
            } else {
                // Persistent mode: use configured store type
                let factory = self
                    .resolve_factory(
                        &self.config.session.store_type,
                        self.config.session.db_config.clone(),
                    )
                    .await?;
                (
                    self.thread_store.unwrap_or_else(|| factory.thread_store()),
                    self.task_store.unwrap_or_else(|| factory.task_store()),
                    self.scratchpad_store
                        .unwrap_or_else(|| factory.scratchpad_store()),
                    self.session_store
                        .unwrap_or_else(|| factory.session_store()),
                )
            };

        // Initialize workflow store (always in-memory for now) if not provided
        let workflow_store = self
            .workflow_store
            .unwrap_or_else(|| Arc::new(InMemoryWorkflowStore::new()) as Arc<dyn WorkflowStore>);

        // Initialize external tool calls store (always in-memory) if not provided
        let external_tool_calls_store = self.external_tool_calls_store.unwrap_or_else(|| {
            Arc::new(InMemoryExternalToolCallsStore::new()) as Arc<dyn ExternalToolCallsStore>
        });

        Ok(InitializedStores {
            session_store,
            agent_store,
            task_store,
            thread_store,
            tool_auth_store,
            scratchpad_store,
            workflow_store,
            memory_store,
            crawl_store: None,
            external_tool_calls_store,
            plugin_store,
            browser_session_store,
            settings_store,
        })
    }
}

/// Helper to initialize a SQLite connection pool
#[cfg(feature = "sqlite")]
pub async fn initialize_sqlite(
    config: Option<DbConnectionConfig>,
) -> anyhow::Result<SqliteStoreBuilder> {
    let config = config.unwrap_or_default();

    let builder = SqliteStoreBuilder::sqlite(&config.database_url, config.max_connections).await?;

    Ok(builder)
}

#[cfg(all(not(feature = "sqlite"), feature = "postgres"))]
pub async fn initialize_postgres(
    config: Option<DbConnectionConfig>,
) -> anyhow::Result<PgStoreBuilder> {
    let config = config.unwrap_or_default();

    let builder = PgStoreBuilder::postgres(&config.database_url, config.max_connections).await?;

    Ok(builder)
}

/// Helper to initialize an ephemeral in-memory SQLite connection
#[cfg(feature = "sqlite")]
pub async fn initialize_ephemeral_sqlite() -> anyhow::Result<SqliteStoreBuilder> {
    // Use shared memory URI with a unique name to ensure:
    // 1. All connections in the pool see the same database
    // 2. Each execution gets an isolated database (unique name)
    // 3. Database persists as long as at least one connection is open
    let db_name = uuid::Uuid::new_v4();
    let database_url = format!("file:{}?mode=memory&cache=shared", db_name);

    tracing::debug!(
        "Creating ephemeral SQLite with shared memory: {}",
        database_url
    );

    let builder = SqliteStoreBuilder::sqlite(&database_url, 5).await?;

    Ok(builder)
}

/// Initialize all stores based on configuration
/// This is a convenience function that uses StoreBuilder internally
pub async fn initialize_stores(config: &StoreConfig) -> anyhow::Result<InitializedStores> {
    StoreBuilder::new(config.clone()).build().await
}

/// Create ephemeral session stores for a single thread execution
/// This is useful for creating isolated, temporary stores that are discarded after execution
pub async fn create_ephemeral_session_stores() -> anyhow::Result<SessionStores> {
    #[cfg(not(feature = "sqlite"))]
    {
        return Err(anyhow!(
            "create_ephemeral_session_stores requires the sqlite feature"
        ));
    }

    #[cfg(feature = "sqlite")]
    {
        tracing::debug!("Creating ephemeral session stores with fresh in-memory database");
        let factory = Arc::new(initialize_ephemeral_sqlite().await?) as Arc<dyn StoreFactory>;

        let thread_store = factory.thread_store();
        let task_store = factory.task_store();
        let scratchpad_store = factory.scratchpad_store();
        let session_store = factory.session_store();

        tracing::debug!("Ephemeral session stores created successfully");

        Ok(SessionStores {
            thread_store,
            task_store,
            scratchpad_store,
            session_store,
        })
    }
}

/// Create full InitializedStores with ephemeral session stores
/// Combines fresh ephemeral session stores with existing persistent stores from base
pub async fn create_ephemeral_execution_stores(
    base_stores: &InitializedStores,
) -> anyhow::Result<InitializedStores> {
    tracing::debug!("Creating execution stores with fresh ephemeral session stores");

    let ephemeral_session = create_ephemeral_session_stores().await?;

    Ok(InitializedStores {
        thread_store: ephemeral_session.thread_store,
        task_store: ephemeral_session.task_store,
        scratchpad_store: ephemeral_session.scratchpad_store,
        session_store: ephemeral_session.session_store,
        // Reuse persistent stores from base
        agent_store: base_stores.agent_store.clone(),
        tool_auth_store: base_stores.tool_auth_store.clone(),
        workflow_store: base_stores.workflow_store.clone(),
        memory_store: base_stores.memory_store.clone(),
        crawl_store: base_stores.crawl_store.clone(),
        external_tool_calls_store: base_stores.external_tool_calls_store.clone(),
        plugin_store: base_stores.plugin_store.clone(),
        browser_session_store: base_stores.browser_session_store.clone(),
        settings_store: base_stores.settings_store.clone(),
    })
}

/// Session stores that can be created per-thread
#[derive(Clone)]
pub struct SessionStores {
    pub thread_store: Arc<dyn ThreadStore>,
    pub task_store: Arc<dyn TaskStore>,
    pub scratchpad_store: Arc<dyn ScratchpadStore>,
    pub session_store: Arc<dyn SessionStore>,
}

/// Prepare stores for execution - just clones the base stores
///
/// For ephemeral mode, the orchestrator's stores are already configured as ephemeral
/// at initialization time, so we just reuse them.
///
/// # Arguments
/// * `base_stores` - The base stores (usually from orchestrator)
/// * `use_ephemeral` - Ignored (kept for API compatibility)
///
/// # Returns
/// Clone of base_stores
pub async fn prepare_stores_for_execution(
    base_stores: &InitializedStores,
    _use_ephemeral: bool,
) -> anyhow::Result<InitializedStores> {
    // Just clone the base stores - they're already configured correctly
    // (ephemeral or persistent based on config)
    Ok(InitializedStores {
        session_store: base_stores.session_store.clone(),
        thread_store: base_stores.thread_store.clone(),
        task_store: base_stores.task_store.clone(),
        scratchpad_store: base_stores.scratchpad_store.clone(),
        agent_store: base_stores.agent_store.clone(),
        tool_auth_store: base_stores.tool_auth_store.clone(),
        workflow_store: base_stores.workflow_store.clone(),
        memory_store: base_stores.memory_store.clone(),
        crawl_store: base_stores.crawl_store.clone(),
        external_tool_calls_store: base_stores.external_tool_calls_store.clone(),
        plugin_store: base_stores.plugin_store.clone(),
        browser_session_store: base_stores.browser_session_store.clone(),
        settings_store: base_stores.settings_store.clone(),
    })
}
