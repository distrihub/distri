use crate::{
    ScratchpadEntry, ToolAuthStore, ToolResponse, configuration::PluginArtifact,
    workflow::WorkflowStore,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::{
    AgentEvent, CreateThreadRequest, Message, Task, TaskMessage, TaskStatus, Thread,
    UpdateThreadRequest,
};

// Redis and PostgreSQL stores moved to distri-stores crate

/// Filter for listing threads
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreadListFilter {
    /// Filter by agent ID
    pub agent_id: Option<String>,
    /// Filter by external ID (for integration with external systems)
    pub external_id: Option<String>,
    /// Filter by thread attributes (JSON matching)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<serde_json::Value>,
    /// Full-text search across title and last_message
    pub search: Option<String>,
    /// Filter threads updated after this time
    pub from_date: Option<DateTime<Utc>>,
    /// Filter threads updated before this time
    pub to_date: Option<DateTime<Utc>>,
    /// Filter by tags (array of tag strings to match)
    pub tags: Option<Vec<String>>,
}

/// Paginated response for thread listing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadListResponse {
    pub threads: Vec<crate::ThreadSummary>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

/// Agent usage information for sorting agents by thread count
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentUsageInfo {
    pub agent_id: String,
    pub agent_name: String,
    pub thread_count: i64,
}

/// Initialized store collection
#[derive(Clone)]
pub struct InitializedStores {
    pub session_store: Arc<dyn SessionStore>,
    pub agent_store: Arc<dyn AgentStore>,
    pub task_store: Arc<dyn TaskStore>,
    pub thread_store: Arc<dyn ThreadStore>,
    pub tool_auth_store: Arc<dyn ToolAuthStore>,
    pub scratchpad_store: Arc<dyn ScratchpadStore>,
    pub workflow_store: Arc<dyn WorkflowStore>,
    pub memory_store: Option<Arc<dyn MemoryStore>>,
    pub crawl_store: Option<Arc<dyn CrawlStore>>,
    pub external_tool_calls_store: Arc<dyn ExternalToolCallsStore>,
    pub plugin_store: Arc<dyn PluginCatalogStore>,
    pub prompt_template_store: Option<Arc<dyn PromptTemplateStore>>,
    pub secret_store: Option<Arc<dyn SecretStore>>,
}
impl InitializedStores {
    pub fn set_tool_auth_store(&mut self, tool_auth_store: Arc<dyn ToolAuthStore>) {
        self.tool_auth_store = tool_auth_store;
    }

    pub fn set_external_tool_calls_store(mut self, store: Arc<dyn ExternalToolCallsStore>) {
        self.external_tool_calls_store = store;
    }

    pub fn set_session_store(&mut self, session_store: Arc<dyn SessionStore>) {
        self.session_store = session_store;
    }

    pub fn set_agent_store(&mut self, agent_store: Arc<dyn AgentStore>) {
        self.agent_store = agent_store;
    }

    pub fn with_task_store(&mut self, task_store: Arc<dyn TaskStore>) {
        self.task_store = task_store;
    }

    pub fn with_thread_store(&mut self, thread_store: Arc<dyn ThreadStore>) {
        self.thread_store = thread_store;
    }

    pub fn with_scratchpad_store(&mut self, scratchpad_store: Arc<dyn ScratchpadStore>) {
        self.scratchpad_store = scratchpad_store;
    }

    pub fn with_workflow_store(mut self, workflow_store: Arc<dyn WorkflowStore>) {
        self.workflow_store = workflow_store;
    }

    pub fn with_plugin_store(&mut self, plugin_store: Arc<dyn PluginCatalogStore>) {
        self.plugin_store = plugin_store;
    }
}

impl std::fmt::Debug for InitializedStores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InitializedStores").finish()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionSummary {
    pub session_id: String,
    pub keys: Vec<String>,
    pub key_count: usize,
    pub updated_at: Option<DateTime<Utc>>,
}

// SessionStore trait - manages current conversation thread/run
#[async_trait::async_trait]
pub trait SessionStore: Send + Sync + std::fmt::Debug {
    async fn clear_session(&self, namespace: &str) -> anyhow::Result<()>;

    async fn set_value(&self, namespace: &str, key: &str, value: &Value) -> anyhow::Result<()>;

    async fn set_value_with_expiry(
        &self,
        namespace: &str,
        key: &str,
        value: &Value,
        expiry: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<()>;

    async fn get_value(&self, namespace: &str, key: &str) -> anyhow::Result<Option<Value>>;

    async fn delete_value(&self, namespace: &str, key: &str) -> anyhow::Result<()>;

    async fn get_all_values(&self, namespace: &str) -> anyhow::Result<HashMap<String, Value>>;

    async fn list_sessions(
        &self,
        namespace: Option<&str>,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> anyhow::Result<Vec<SessionSummary>>;
}
#[async_trait::async_trait]
pub trait SessionStoreExt: SessionStore {
    async fn set<T: Serialize + Sync>(
        &self,
        namespace: &str,
        key: &str,
        value: &T,
    ) -> anyhow::Result<()> {
        self.set_value(namespace, key, &serde_json::to_value(value)?)
            .await
    }
    async fn set_with_expiry<T: Serialize + Sync>(
        &self,
        namespace: &str,
        key: &str,
        value: &T,
        expiry: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<()> {
        self.set_value_with_expiry(namespace, key, &serde_json::to_value(value)?, expiry)
            .await
    }
    async fn get<T: DeserializeOwned + Sync>(
        &self,
        namespace: &str,
        key: &str,
    ) -> anyhow::Result<Option<T>> {
        match self.get_value(namespace, key).await? {
            Some(b) => Ok(Some(serde_json::from_value(b)?)),
            None => Ok(None),
        }
    }
}
impl<T: SessionStore + ?Sized> SessionStoreExt for T {}

// Higher-level MemoryStore trait - manages cross-session permanent memory using user_id
#[async_trait::async_trait]
pub trait MemoryStore: Send + Sync {
    /// Store permanent memory from a session for cross-session access
    async fn store_memory(
        &self,
        user_id: &str,
        session_memory: SessionMemory,
    ) -> anyhow::Result<()>;

    /// Search for relevant memories across sessions for a user
    async fn search_memories(
        &self,
        user_id: &str,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<String>>;

    /// Get all permanent memories for a user
    async fn get_user_memories(&self, user_id: &str) -> anyhow::Result<Vec<String>>;

    /// Clear all memories for a user
    async fn clear_user_memories(&self, user_id: &str) -> anyhow::Result<()>;
}

#[derive(Debug, Clone)]
pub struct SessionMemory {
    pub agent_id: String,
    pub thread_id: String,
    pub session_summary: String,
    pub key_insights: Vec<String>,
    pub important_facts: Vec<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FilterMessageType {
    Events,
    Messages,
    Artifacts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageFilter {
    pub filter: Option<Vec<FilterMessageType>>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

// Task Store trait for A2A task management
#[async_trait]
pub trait TaskStore: Send + Sync {
    fn init_task(
        &self,
        context_id: &str,
        task_id: Option<&str>,
        status: Option<TaskStatus>,
    ) -> Task {
        let task_id = task_id.unwrap_or(&Uuid::new_v4().to_string()).to_string();
        Task {
            id: task_id,
            status: status.unwrap_or(TaskStatus::Pending),
            created_at: chrono::Utc::now().timestamp_millis(),
            updated_at: chrono::Utc::now().timestamp_millis(),
            thread_id: context_id.to_string(),
            parent_task_id: None,
        }
    }
    async fn get_or_create_task(
        &self,
        thread_id: &str,
        task_id: &str,
    ) -> Result<(), anyhow::Error> {
        match self.get_task(&task_id).await? {
            Some(task) => task,
            None => {
                self.create_task(&thread_id, Some(&task_id), Some(TaskStatus::Running))
                    .await?
            }
        };

        Ok(())
    }
    async fn create_task(
        &self,
        context_id: &str,
        task_id: Option<&str>,
        task_status: Option<TaskStatus>,
    ) -> anyhow::Result<Task>;
    async fn get_task(&self, task_id: &str) -> anyhow::Result<Option<Task>>;
    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> anyhow::Result<()>;
    async fn add_event_to_task(&self, task_id: &str, event: AgentEvent) -> anyhow::Result<()>;
    async fn add_message_to_task(&self, task_id: &str, message: &Message) -> anyhow::Result<()>;
    async fn cancel_task(&self, task_id: &str) -> anyhow::Result<Task>;
    async fn list_tasks(&self, thread_id: Option<&str>) -> anyhow::Result<Vec<Task>>;

    async fn get_history(
        &self,
        thread_id: &str,
        filter: Option<MessageFilter>,
    ) -> anyhow::Result<Vec<(Task, Vec<TaskMessage>)>>;

    async fn update_parent_task(
        &self,
        task_id: &str,
        parent_task_id: Option<&str>,
    ) -> anyhow::Result<()>;
}

#[derive(Debug, Clone)]
pub struct PluginMetadataRecord {
    pub package_name: String,
    pub version: Option<String>,
    pub object_prefix: String,
    pub entrypoint: Option<String>,
    pub artifact: PluginArtifact,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[async_trait]
pub trait PluginCatalogStore: Send + Sync {
    async fn list_plugins(&self) -> anyhow::Result<Vec<PluginMetadataRecord>>;

    async fn get_plugin(&self, package_name: &str) -> anyhow::Result<Option<PluginMetadataRecord>>;

    async fn upsert_plugin(&self, record: &PluginMetadataRecord) -> anyhow::Result<()>;

    async fn remove_plugin(&self, package_name: &str) -> anyhow::Result<()>;

    async fn clear(&self) -> anyhow::Result<()>;
}

// Thread Store trait for thread management
#[async_trait]
pub trait ThreadStore: Send + Sync {
    fn as_any(&self) -> &dyn std::any::Any;
    async fn create_thread(&self, request: CreateThreadRequest) -> anyhow::Result<Thread>;
    async fn get_thread(&self, thread_id: &str) -> anyhow::Result<Option<Thread>>;
    async fn update_thread(
        &self,
        thread_id: &str,
        request: UpdateThreadRequest,
    ) -> anyhow::Result<Thread>;
    async fn delete_thread(&self, thread_id: &str) -> anyhow::Result<()>;

    /// List threads with pagination and filtering
    /// Returns a paginated response with total count
    async fn list_threads(
        &self,
        filter: &ThreadListFilter,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> anyhow::Result<ThreadListResponse>;

    async fn update_thread_with_message(
        &self,
        thread_id: &str,
        message: &str,
    ) -> anyhow::Result<()>;

    /// Get aggregated home statistics
    async fn get_home_stats(&self) -> anyhow::Result<HomeStats>;

    /// Get agents sorted by thread count (most active first)
    async fn get_agents_by_usage(&self) -> anyhow::Result<Vec<AgentUsageInfo>>;

    /// Get a map of agent name -> stats for all agents with activity
    async fn get_agent_stats_map(
        &self,
    ) -> anyhow::Result<std::collections::HashMap<String, AgentStatsInfo>>;
}

/// Home statistics for dashboard
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HomeStats {
    pub total_agents: i64,
    pub total_threads: i64,
    pub total_messages: i64,
    pub avg_run_time_ms: Option<f64>,
    // Cloud-specific fields (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_owned_agents: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_accessible_agents: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub most_active_agent: Option<MostActiveAgent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_threads: Option<Vec<LatestThreadInfo>>,
    /// Recently used agents (last 10 by most recent thread activity)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recently_used_agents: Option<Vec<RecentlyUsedAgent>>,
    /// Custom metrics that can be displayed in the stats overview
    /// Key is the metric name (e.g., "usage"), value is the metric data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_metrics: Option<std::collections::HashMap<String, CustomMetric>>,
}

/// A custom metric for display in the stats overview
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CustomMetric {
    /// Display label (e.g., "Monthly Calls")
    pub label: String,
    /// Current value as a string (formatted)
    pub value: String,
    /// Optional helper text below the value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub helper: Option<String>,
    /// Optional limit (for progress display)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<String>,
    /// Optional raw numeric value for calculations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_value: Option<i64>,
    /// Optional raw limit for calculations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_limit: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MostActiveAgent {
    pub id: String,
    pub name: String,
    pub thread_count: i64,
}

/// Agent that was recently used (based on thread activity)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecentlyUsedAgent {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub last_used_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LatestThreadInfo {
    pub id: String,
    pub title: String,
    pub agent_id: String,
    pub agent_name: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Agent statistics for display
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AgentStatsInfo {
    pub thread_count: i64,
    pub sub_agent_usage_count: i64,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[async_trait]
pub trait AgentStore: Send + Sync {
    async fn list(
        &self,
        cursor: Option<String>,
        limit: Option<usize>,
    ) -> (Vec<crate::configuration::AgentConfig>, Option<String>);

    async fn get(&self, name: &str) -> Option<crate::configuration::AgentConfig>;
    async fn register(&self, config: crate::configuration::AgentConfig) -> anyhow::Result<()>;
    /// Update an existing agent with new definition
    async fn update(&self, config: crate::configuration::AgentConfig) -> anyhow::Result<()>;

    async fn clear(&self) -> anyhow::Result<()>;
}

/// Store for managing scratchpad entries across conversations
#[async_trait::async_trait]
pub trait ScratchpadStore: Send + Sync + std::fmt::Debug {
    /// Add a scratchpad entry for a specific thread
    async fn add_entry(
        &self,
        thread_id: &str,
        entry: ScratchpadEntry,
    ) -> Result<(), crate::AgentError>;

    /// Clear all scratchpad entries for a thread
    async fn clear_entries(&self, thread_id: &str) -> Result<(), crate::AgentError>;

    /// Get entries for a specific task within a thread
    async fn get_entries(
        &self,
        thread_id: &str,
        task_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ScratchpadEntry>, crate::AgentError>;

    async fn get_all_entries(
        &self,
        thread_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ScratchpadEntry>, crate::AgentError>;
}

/// Web crawl result data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlResult {
    pub id: String,
    pub url: String,
    pub title: Option<String>,
    pub content: String,
    pub html: Option<String>,
    pub metadata: serde_json::Value,
    pub links: Vec<String>,
    pub images: Vec<String>,
    pub status_code: Option<u16>,
    pub crawled_at: chrono::DateTime<chrono::Utc>,
    pub processing_time_ms: Option<u64>,
}

/// Store for managing web crawl results
#[async_trait]
pub trait CrawlStore: Send + Sync {
    /// Store a crawl result
    async fn store_crawl_result(&self, result: CrawlResult) -> anyhow::Result<String>;

    /// Get a crawl result by ID
    async fn get_crawl_result(&self, id: &str) -> anyhow::Result<Option<CrawlResult>>;

    /// Get crawl results for a specific URL
    async fn get_crawl_results_by_url(&self, url: &str) -> anyhow::Result<Vec<CrawlResult>>;

    /// Get recent crawl results (within time limit)
    async fn get_recent_crawl_results(
        &self,
        limit: Option<usize>,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Vec<CrawlResult>>;

    /// Check if URL was recently crawled (within cache duration)
    async fn is_url_recently_crawled(
        &self,
        url: &str,
        cache_duration: chrono::Duration,
    ) -> anyhow::Result<Option<CrawlResult>>;

    /// Delete crawl result
    async fn delete_crawl_result(&self, id: &str) -> anyhow::Result<()>;

    /// Clear all crawl results older than specified date
    async fn cleanup_old_results(
        &self,
        before: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<usize>;
}

/// Store for managing external tool call completions using oneshot channels
#[async_trait]
pub trait ExternalToolCallsStore: Send + Sync + std::fmt::Debug {
    /// Register a new external tool call session and return a receiver for the response
    async fn register_external_tool_call(
        &self,
        session_id: &str,
    ) -> anyhow::Result<oneshot::Receiver<ToolResponse>>;

    /// Complete an external tool call by sending the response
    async fn complete_external_tool_call(
        &self,
        session_id: &str,
        tool_response: ToolResponse,
    ) -> anyhow::Result<()>;

    /// Remove a session (cleanup)
    async fn remove_tool_call(&self, session_id: &str) -> anyhow::Result<()>;

    /// List all pending sessions (for debugging)
    async fn list_pending_tool_calls(&self) -> anyhow::Result<Vec<String>>;
}

// ========== Prompt Template Store ==========

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplateRecord {
    pub id: String,
    pub name: String,
    pub template: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub is_system: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewPromptTemplate {
    pub name: String,
    pub template: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub is_system: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePromptTemplate {
    pub name: String,
    pub template: String,
    pub description: Option<String>,
}

#[async_trait]
pub trait PromptTemplateStore: Send + Sync {
    async fn list(&self) -> anyhow::Result<Vec<PromptTemplateRecord>>;
    async fn get(&self, id: &str) -> anyhow::Result<Option<PromptTemplateRecord>>;
    async fn create(&self, template: NewPromptTemplate) -> anyhow::Result<PromptTemplateRecord>;
    async fn update(
        &self,
        id: &str,
        update: UpdatePromptTemplate,
    ) -> anyhow::Result<PromptTemplateRecord>;
    async fn delete(&self, id: &str) -> anyhow::Result<()>;
    async fn clone_template(&self, id: &str) -> anyhow::Result<PromptTemplateRecord>;
    async fn sync_system_templates(&self, templates: Vec<NewPromptTemplate>) -> anyhow::Result<()>;
}

// ========== Secret Store ==========

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretRecord {
    pub id: String,
    pub key: String,
    pub value: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewSecret {
    pub key: String,
    pub value: String,
}

#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn list(&self) -> anyhow::Result<Vec<SecretRecord>>;
    async fn get(&self, key: &str) -> anyhow::Result<Option<SecretRecord>>;
    async fn create(&self, secret: NewSecret) -> anyhow::Result<SecretRecord>;
    async fn update(&self, key: &str, value: &str) -> anyhow::Result<SecretRecord>;
    async fn delete(&self, key: &str) -> anyhow::Result<()>;
}
