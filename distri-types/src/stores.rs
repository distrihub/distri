use crate::connections::{Connection, ConnectionStatus, ConnectionToken, NewConnection};
use crate::{ScratchpadEntry, ToolAuthStore, ToolResponse};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::oneshot;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    AgentEvent, CreateThreadRequest, Message, Task, TaskMessage, TaskStatus, Thread,
    UpdateThreadRequest,
};

/// Inputs to [`TaskStore::create_task`]. The schema records only
/// (`remote`, `inner_task_id`, `ended_at`, `invocation`) for the
/// Invocation half — `remote` is the fast filter, the typed Executor +
/// RunnerConfig live inside `invocation`. Adding a new runner kind
/// (k8s, fly, …) does NOT require a schema migration; ship a new
/// [`RunnerInitializer`] keyed by [`RunnerConfig::kind`].
///
/// Use [`CreateTaskInput::local`] for a local-executor task; chain
/// [`CreateTaskInput::with_remote`] to flip `remote` to true and set
/// `inner_task_id` together.
#[derive(Debug, Clone)]
pub struct CreateTaskInput {
    pub thread_id: String,
    pub task_id: Option<String>,
    pub status: Option<TaskStatus>,
    pub parent_task_id: Option<String>,
    /// `true` when another orchestrator runs the loop. Equivalent to
    /// `Executor::Remote { runner }` in the in-memory `Invocation`. The
    /// runner kind/config is NOT denormalized into the schema — it
    /// lives only in the `invocation` blob, dispatched at runtime via
    /// the `RunnerInitializer` registry.
    pub remote: bool,
    /// task_id on the inner orchestrator. Must be `None` when
    /// `remote == false` (DB CHECK enforces). May be `None`
    /// transiently for remote rows — between row insert and the
    /// runner assigning its inner id.
    pub inner_task_id: Option<String>,
    /// Serialized [`Invocation`](crate::invocation::Invocation). The
    /// canonical record of what was requested, including the typed
    /// `Executor` and any `RunnerConfig`. Stored as JSONB in Pg /
    /// TEXT in sqlite. Default is `{}` until invoke() is wired.
    pub invocation: serde_json::Value,
}

impl CreateTaskInput {
    /// Local-executor task. `task_id` / `status` / `parent_task_id` /
    /// `invocation` are chained via the `with_*` builders.
    pub fn local(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            task_id: None,
            status: None,
            parent_task_id: None,
            remote: false,
            inner_task_id: None,
            invocation: serde_json::Value::Object(Default::default()),
        }
    }

    pub fn with_id(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    pub fn with_status(mut self, status: TaskStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_parent(mut self, parent_task_id: impl Into<String>) -> Self {
        self.parent_task_id = Some(parent_task_id.into());
        self
    }

    pub fn with_invocation(mut self, invocation: serde_json::Value) -> Self {
        self.invocation = invocation;
        self
    }

    /// Marks the task as remote-executed and sets the inner task id
    /// the runner has assigned. The runner kind + its private config
    /// live in `invocation` (typed `Executor::Remote { runner }`).
    pub fn with_remote(mut self, inner_task_id: impl Into<String>) -> Self {
        self.remote = true;
        self.inner_task_id = Some(inner_task_id.into());
        self
    }
}

// Redis and PostgreSQL stores moved to distri-stores crate

/// Filter for listing threads
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ThreadListResponse {
    pub threads: Vec<crate::ThreadSummary>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

/// Agent usage information for sorting agents by thread count
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
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
    pub memory_store: Option<Arc<dyn MemoryStore>>,
    pub crawl_store: Option<Arc<dyn CrawlStore>>,
    pub external_tool_calls_store: Arc<dyn ExternalToolCallsStore>,
    pub prompt_template_store: Option<Arc<dyn PromptTemplateStore>>,
    pub secret_store: Option<Arc<dyn SecretStore>>,
    pub skill_store: Option<Arc<dyn SkillStore>>,
    pub connection_store: Option<Arc<dyn ConnectionStore>>,
    pub connection_token_store: Option<Arc<dyn ConnectionTokenStore>>,
    pub provider_registry: Option<Arc<dyn crate::auth::ProviderRegistry>>,
    pub span_store: Option<Arc<dyn SpanStore>>,
    pub note_store: Option<Arc<dyn NoteStore>>,
    /// Provider settings store (`/v1/providers` routes). `None` for the
    /// multi-tenant cloud, which registers a workspace-scoped `ProviderStore`
    /// separately rather than through `InitializedStores`.
    pub provider_store: Option<Arc<dyn ProviderStore>>,
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
}

impl std::fmt::Debug for InitializedStores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InitializedStores").finish()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FilterMessageType {
    Events,
    Messages,
    Artifacts,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct MessageFilter {
    pub filter: Option<Vec<FilterMessageType>>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

// Task Store trait for A2A task management
#[async_trait]
pub trait TaskStore: Send + Sync {
    /// Build (but do not persist) a Task row from a [`CreateTaskInput`]. The
    /// returned struct is what the implementations will hand back from
    /// `create_task` once the row has been inserted.
    fn init_task(&self, input: &CreateTaskInput) -> Task {
        let task_id = input
            .task_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        Task {
            id: task_id,
            status: input.status.clone().unwrap_or(TaskStatus::Pending),
            created_at: chrono::Utc::now().timestamp_millis(),
            updated_at: chrono::Utc::now().timestamp_millis(),
            thread_id: input.thread_id.clone(),
            parent_task_id: input.parent_task_id.clone(),
        }
    }

    async fn get_or_create_task(
        &self,
        thread_id: &str,
        task_id: &str,
    ) -> Result<(), anyhow::Error> {
        match self.get_task(task_id).await? {
            Some(task) => task,
            None => {
                self.create_task(
                    CreateTaskInput::local(thread_id)
                        .with_id(task_id)
                        .with_status(TaskStatus::Running),
                )
                .await?
            }
        };

        Ok(())
    }

    async fn create_task(&self, input: CreateTaskInput) -> anyhow::Result<Task>;
    async fn get_task(&self, task_id: &str) -> anyhow::Result<Option<Task>>;
    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> anyhow::Result<()>;
    async fn add_event_to_task(&self, task_id: &str, event: AgentEvent) -> anyhow::Result<()>;
    async fn add_message_to_task(&self, task_id: &str, message: &Message) -> anyhow::Result<()>;
    async fn cancel_task(&self, task_id: &str) -> anyhow::Result<Task>;

    /// Cancel `root_task_id` and every task whose `parent_task_id` chain
    /// leads back to it, in one transaction.
    ///
    /// Idempotent on terminal rows: tasks already in `Completed`, `Failed`,
    /// or `Canceled` are left untouched. The returned `Vec<Task>` contains
    /// the rows that were actually transitioned to `Canceled` — the caller
    /// uses this to publish corresponding cancel events on the broadcaster
    /// so live in-process loops can stop.
    ///
    /// The cascade is implemented via a recursive CTE on the `parent_task_id`
    /// edge; the `idx_tasks_parent_id` index keeps the walk cheap.
    async fn cancel_task_cascade(&self, root_task_id: &str) -> anyhow::Result<Vec<Task>>;

    /// Read-only walk of the parent_task_id graph rooted at `root_task_id`,
    /// returning the root + every descendant. Used by the `list_my_tasks`
    /// supervisor tool when scoped to a sub-tree, and by `wait_task` to
    /// wait on the whole sub-tree of a Detached invocation.
    ///
    /// Order is breadth-first by descendant depth (root first); within a
    /// level the order is implementation-defined.
    async fn list_descendant_tasks(&self, root_task_id: &str) -> anyhow::Result<Vec<Task>>;

    /// All non-terminal tasks. When `thread_id` is `Some`, scopes to that
    /// thread (inputs of `list_my_tasks` from a thread-scoped supervisor);
    /// otherwise returns every running task visible to the caller (cloud
    /// tenant isolation still applies).
    ///
    /// "Running" here means the schema status `running` — tasks in `pending`,
    /// `input_required`, or terminal states are excluded. The partial index
    /// `idx_tasks_running` covers this query.
    async fn list_running_tasks(&self, thread_id: Option<&str>) -> anyhow::Result<Vec<Task>>;
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

    /// Persist the latest `ContextBudget` snapshot (as a pre-serialized JSON
    /// value) on the thread row. Written by the orchestrator's task relay on
    /// every `ContextBudgetUpdate` event so non-live surfaces can render the
    /// breakdown. Default impl is a no-op so tests / in-memory stores don't
    /// need to care.
    async fn update_last_context_budget(
        &self,
        thread_id: &str,
        budget: Option<serde_json::Value>,
    ) -> anyhow::Result<()> {
        let _ = (thread_id, budget);
        Ok(())
    }

    /// Get aggregated home statistics
    async fn get_home_stats(&self) -> anyhow::Result<HomeStats>;

    /// Get agents sorted by thread count (most active first)
    /// Includes all registered agents (even those with 0 threads).
    /// Optionally filters by name using a search string.
    async fn get_agents_by_usage(
        &self,
        search: Option<&str>,
    ) -> anyhow::Result<Vec<AgentUsageInfo>>;

    /// Get a map of agent name -> stats for all agents with activity
    async fn get_agent_stats_map(
        &self,
    ) -> anyhow::Result<std::collections::HashMap<String, AgentStatsInfo>>;

    // ========== Message Read Status Methods ==========

    /// Mark a message as read by the current user
    async fn mark_message_read(
        &self,
        thread_id: &str,
        message_id: &str,
    ) -> anyhow::Result<MessageReadStatus>;

    /// Get read status for a specific message
    async fn get_message_read_status(
        &self,
        thread_id: &str,
        message_id: &str,
    ) -> anyhow::Result<Option<MessageReadStatus>>;

    /// Get read status for all messages in a thread for the current user
    async fn get_thread_read_status(
        &self,
        thread_id: &str,
    ) -> anyhow::Result<Vec<MessageReadStatus>>;

    // ========== Message Voting Methods ==========

    /// Vote on a message (upvote or downvote)
    /// For downvotes, a comment is required
    async fn vote_message(&self, request: VoteMessageRequest) -> anyhow::Result<MessageVote>;

    /// Remove a vote from a message
    async fn remove_vote(&self, thread_id: &str, message_id: &str) -> anyhow::Result<()>;

    /// Get the current user's vote on a message
    async fn get_user_vote(
        &self,
        thread_id: &str,
        message_id: &str,
    ) -> anyhow::Result<Option<MessageVote>>;

    /// Get vote summary for a message (counts + current user's vote)
    async fn get_message_vote_summary(
        &self,
        thread_id: &str,
        message_id: &str,
    ) -> anyhow::Result<MessageVoteSummary>;

    /// Get all votes for a message (admin/analytics use)
    async fn get_message_votes(
        &self,
        thread_id: &str,
        message_id: &str,
    ) -> anyhow::Result<Vec<MessageVote>>;
}

/// Home statistics for dashboard
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema, JsonSchema)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema, JsonSchema)]
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema, JsonSchema)]
pub struct MostActiveAgent {
    pub id: String,
    pub name: String,
    pub thread_count: i64,
}

/// Agent that was recently used (based on thread activity)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema, JsonSchema)]
pub struct RecentlyUsedAgent {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub last_used_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema, JsonSchema)]
pub struct LatestThreadInfo {
    pub id: String,
    pub title: String,
    pub agent_id: String,
    pub agent_name: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Agent statistics for display
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, ToSchema, JsonSchema)]
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

    /// Delete an agent by name or ID
    async fn delete(&self, id: &str) -> anyhow::Result<()>;

    /// Get an agent with cloud-specific metadata (id, published, is_owner, etc.)
    /// Default impl returns empty metadata — override in cloud stores.
    async fn get_with_cloud_metadata(
        &self,
        name: &str,
    ) -> Option<(
        crate::configuration::AgentConfig,
        crate::configuration::AgentCloudMetadata,
    )> {
        self.get(name)
            .await
            .map(|c| (c, crate::configuration::AgentCloudMetadata::default()))
    }

    /// List agents with cloud-specific metadata.
    /// Default impl returns empty metadata — override in cloud stores.
    async fn list_with_cloud_metadata(
        &self,
        cursor: Option<String>,
        limit: Option<usize>,
    ) -> (
        Vec<(
            crate::configuration::AgentConfig,
            crate::configuration::AgentCloudMetadata,
        )>,
        Option<String>,
    ) {
        let (configs, cursor) = self.list(cursor, limit).await;
        (
            configs
                .into_iter()
                .map(|c| (c, crate::configuration::AgentCloudMetadata::default()))
                .collect(),
            cursor,
        )
    }
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
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
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

// ========== Message Read & Voting Types ==========

/// Vote type for message feedback
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum VoteType {
    Upvote,
    Downvote,
}

/// Record of a message being read
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct MessageReadStatus {
    pub thread_id: String,
    pub message_id: String,
    pub user_id: String,
    pub read_at: chrono::DateTime<chrono::Utc>,
}

/// Request to mark a message as read
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct MarkMessageReadRequest {
    pub thread_id: String,
    pub message_id: String,
}

/// A vote on a message with optional feedback comment
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct MessageVote {
    pub id: String,
    pub thread_id: String,
    pub message_id: String,
    pub user_id: String,
    pub vote_type: VoteType,
    /// Comment is required for downvotes, optional for upvotes
    pub comment: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Request to vote on a message
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[schema(example = json!({"vote_type": "up"}))]
pub struct VoteMessageRequest {
    pub thread_id: String,
    pub message_id: String,
    pub vote_type: VoteType,
    /// Required for downvotes
    pub comment: Option<String>,
}

/// Summary of votes for a message
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema, JsonSchema)]
pub struct MessageVoteSummary {
    pub message_id: String,
    pub upvotes: i64,
    pub downvotes: i64,
    /// Current user's vote on this message, if any
    pub user_vote: Option<VoteType>,
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[schema(example = json!({"name": "greeting", "content": "Hello {{name}}, welcome to {{service}}!", "description": "A greeting template"}))]
pub struct NewPromptTemplate {
    pub name: String,
    pub template: String,
    pub description: Option<String>,
    pub version: Option<String>,
    #[serde(default)]
    pub is_system: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpdatePromptTemplate {
    pub name: String,
    pub template: String,
    pub description: Option<String>,
}

#[async_trait]
pub trait PromptTemplateStore: Send + Sync {
    async fn list(&self) -> anyhow::Result<Vec<PromptTemplateRecord>>;
    async fn get(&self, id: &str) -> anyhow::Result<Option<PromptTemplateRecord>>;
    /// Fetch multiple templates by name in a single query.
    async fn get_by_names(&self, names: &[String]) -> anyhow::Result<Vec<PromptTemplateRecord>>;
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SecretRecord {
    pub id: String,
    pub key: String,
    pub value: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[schema(example = json!({"key": "OPENAI_API_KEY", "value": "sk-..."}))]
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

// ========== Provider Store ==========

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct CustomProviderConfig {
    pub id: String,
    pub name: String,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct CustomModelEntry {
    pub provider: String,
    pub model: String,
    /// "completion" (default), "tts", or "stt"
    #[serde(default = "default_completion")]
    pub capability: String,
}

fn default_completion() -> String {
    "completion".to_string()
}

/// A custom connection provider (OAuth integration) stored in workspace settings.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ConnectionProviderConfig {
    /// Unique identifier (e.g., "linear", "figma", "custom_crm")
    pub id: String,
    /// Display name
    pub name: String,
    /// OAuth2 authorization URL
    pub authorization_url: String,
    /// OAuth2 token URL
    pub token_url: String,
    /// Optional refresh URL (defaults to token_url)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_url: Option<String>,
    /// Scopes the provider supports
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    /// Default scopes to request
    #[serde(default)]
    pub default_scopes: Vec<String>,
    /// Friendly scope name → full scope string mappings
    #[serde(default)]
    pub scope_mappings: std::collections::HashMap<String, String>,
}

/// Request payload for upserting a provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpsertProviderRequest {
    pub provider_id: String,
    #[serde(default)]
    pub secrets: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub config: Option<CustomProviderConfig>,
    #[serde(default)]
    pub custom_models: Option<Vec<CustomModelEntry>>,
    /// Default model in "provider/model" format. Empty string or null to clear.
    #[serde(default)]
    pub default_model: Option<String>,
    /// Connection provider config (OAuth integration) to add/update.
    #[serde(default)]
    pub connection_provider: Option<ConnectionProviderConfig>,
}

/// Response after upserting a provider.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpsertProviderResponse {
    pub provider_id: String,
    pub secrets_saved: usize,
    pub config_saved: bool,
}

/// Request to validate an already-configured provider — `POST
/// /v1/providers/test`. Credentials are resolved from stored config
/// server-side; nothing sensitive is sent over the wire.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TestProviderRequest {
    /// Built-in provider id (e.g. `azure_ai_foundry`) or a custom provider id.
    pub provider_id: String,
}

/// A provider's resolved probe target — the OpenAI-compatible base URL and
/// API key from stored config. Internal (store → route), not a wire type.
#[derive(Debug, Clone)]
pub struct ResolvedProviderEndpoint {
    pub base_url: String,
    pub api_key: String,
}

/// Result of a `POST /v1/providers/test` probe.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TestProviderResponse {
    /// True when the endpoint answered `GET /models` successfully.
    pub ok: bool,
    /// Model ids the endpoint reported, when reachable.
    #[serde(default)]
    pub models: Vec<String>,
    /// Failure detail when `ok == false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[async_trait]
pub trait ProviderStore: Send + Sync {
    async fn upsert_provider(
        &self,
        req: UpsertProviderRequest,
    ) -> anyhow::Result<UpsertProviderResponse>;

    async fn delete_provider(&self, provider_id: &str) -> anyhow::Result<()>;

    async fn get_default_model(&self) -> anyhow::Result<Option<String>>;

    /// Resolve a provider's probe target (base URL + API key) from stored
    /// config, for the `POST /v1/providers/test` validation endpoint.
    async fn resolve_provider_endpoint(
        &self,
        provider_id: &str,
    ) -> anyhow::Result<ResolvedProviderEndpoint>;
}

/// Resolve a provider's `(base_url, api_key)` for the `/providers/test`
/// probe. Built-in providers hydrate from their canonical secret keys;
/// custom providers take `base_url` from `custom_providers` and the key
/// from `{PROVIDER_ID}_API_KEY`. Shared by the cloud and standalone stores.
pub async fn resolve_provider_test_endpoint(
    provider_id: &str,
    secret_store: &dyn SecretStore,
    custom_providers: &[CustomProviderConfig],
) -> anyhow::Result<ResolvedProviderEndpoint> {
    use crate::agent::ModelSettings;
    // Built-in provider: build its ModelProvider and hydrate from secrets.
    if let Ok(Some(mut ms)) = ModelSettings::from_provider_model_str(&format!("{provider_id}/_")) {
        ms.hydrate_creds(secret_store)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        let (base_url, api_key) = ms.inner.provider.resolved_endpoint();
        let base_url = base_url.filter(|u| !u.trim().is_empty()).ok_or_else(|| {
            anyhow::anyhow!("provider '{provider_id}' has no endpoint configured")
        })?;
        return Ok(ResolvedProviderEndpoint {
            base_url,
            api_key: api_key.unwrap_or_default(),
        });
    }
    // Custom OpenAI-compatible provider.
    let cp = custom_providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| anyhow::anyhow!("unknown provider '{provider_id}'"))?;
    let api_key = secret_store
        .get(&format!("{}_API_KEY", provider_id.to_uppercase()))
        .await
        .map_err(|e| anyhow::anyhow!(e))?
        .map(|s| s.value)
        .unwrap_or_default();
    Ok(ResolvedProviderEndpoint {
        base_url: cp.base_url.clone(),
        api_key,
    })
}

/// Provider-related settings for the single-tenant standalone server.
///
/// Persisted as the `config_json` of the one `server_settings` row. This
/// mirrors the provider-relevant subset of the cloud's per-workspace
/// `WorkspaceSettings` — the standalone server has no `workspaces` table,
/// so there is exactly one of these.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ServerSettings {
    /// Default model in `"provider/model"` format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    /// Custom (non-built-in) provider definitions.
    #[serde(default)]
    pub custom_providers: Vec<CustomProviderConfig>,
    /// Custom model entries, each keyed to a provider id.
    #[serde(default)]
    pub custom_models: Vec<CustomModelEntry>,
    /// Connection (OAuth) provider definitions.
    #[serde(default)]
    pub connection_providers: Vec<ConnectionProviderConfig>,
}

// ========== Skill Store ==========

/// How a skill is executed relative to the calling agent's context.
/// Mirrors the `context` field in claude-code's prompt command spec.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, ToSchema, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ContextExecutionType {
    /// Inject the full skill content into the current agent's context window.
    /// The calling agent incorporates it directly — no sub-agent spawned.
    #[default]
    Inline,
    /// Spawn an isolated child agent with the skill as its instruction set.
    /// The child runs with its own token budget and task record; its result
    /// is summarised and returned to the parent.
    Fork,
}

impl std::fmt::Display for ContextExecutionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextExecutionType::Inline => write!(f, "inline"),
            ContextExecutionType::Fork => write!(f, "fork"),
        }
    }
}

impl std::str::FromStr for ContextExecutionType {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fork" => Ok(ContextExecutionType::Fork),
            _ => Ok(ContextExecutionType::Inline),
        }
    }
}

/// Total token budget for all skill listings in the system prompt.
pub const SKILL_LISTING_BUDGET: usize = 2_000;
/// Max description chars per skill in system prompt listing.
pub const SKILL_DESCRIPTION_CAP: usize = 250;
/// Default max output tokens for a skill when not explicitly set.
pub const DEFAULT_SKILL_MAX_TOKENS: u32 = 8000;

/// Parsed frontmatter from a SKILL.md file (agentskills.io spec).
///
/// Per the spec at https://agentskills.io/specification:
/// - `name` — required, lowercase a-z + hyphens, must match parent directory.
/// - `description` — required.
/// - `license` — optional license name or path.
/// - `compatibility` — optional environment requirements string.
/// - `metadata` — optional free-form key/value map (where distri-specific
///    knobs like `model`, `max_tokens`, `can_spawn_tasks` live).
/// - `allowed_tools` — optional pre-approved tools list (experimental).
///
/// All distri-specific runtime hints are read from `metadata` so the file
/// is portable to any agentskills.io-compliant client.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema, JsonSchema)]
pub struct SkillFrontmatter {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub metadata: std::collections::HashMap<String, String>,
    /// Maps to `allowed-tools` on the wire (per agentskills.io spec).
    #[serde(
        default,
        rename = "allowed-tools",
        skip_serializing_if = "Option::is_none"
    )]
    pub allowed_tools: Option<String>,
}

impl SkillFrontmatter {
    /// Distri-specific runtime hint stored in `metadata.model`.
    pub fn model(&self) -> Option<&str> {
        self.metadata.get("model").map(|s| s.as_str())
    }

    /// Distri-specific runtime hint stored in `metadata.max_tokens`.
    pub fn max_tokens(&self) -> Option<u32> {
        self.metadata.get("max_tokens").and_then(|s| s.parse().ok())
    }

    /// Distri-specific runtime hint stored in `metadata.can_spawn_tasks`.
    pub fn can_spawn_tasks(&self) -> bool {
        self.metadata
            .get("can_spawn_tasks")
            .map(|s| s == "true" || s == "yes")
            .unwrap_or(false)
    }

    /// Distri-specific runtime hint stored as a comma- or space-separated list.
    pub fn tags(&self) -> Vec<String> {
        self.metadata
            .get("tags")
            .map(|s| {
                s.split(|c: char| c == ',' || c.is_whitespace())
                    .filter(|t| !t.is_empty())
                    .map(|t| t.trim().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn effective_max_tokens(&self) -> u32 {
        self.max_tokens().unwrap_or(DEFAULT_SKILL_MAX_TOKENS)
    }

    pub fn as_listing_line(&self) -> String {
        let desc = self.description.as_deref().unwrap_or("No description");
        let desc_truncated = if desc.len() > SKILL_DESCRIPTION_CAP {
            format!("{}...", &desc[..SKILL_DESCRIPTION_CAP.min(desc.len())])
        } else {
            desc.to_string()
        };
        let mut meta = Vec::new();
        if let Some(model) = self.model() {
            meta.push(format!("model: {}", model));
        }
        if self.can_spawn_tasks() {
            meta.push("tasks: yes".to_string());
        }
        if meta.is_empty() {
            format!("- {}: {}", self.name, desc_truncated)
        } else {
            format!("- {}: {} ({})", self.name, desc_truncated, meta.join(", "))
        }
    }
}

/// Format a list of skills for the system prompt, respecting a token budget.
pub fn format_skill_listing(skills: &[SkillFrontmatter], budget_tokens: usize) -> String {
    let budget_chars = budget_tokens * 4;
    let mut result = String::new();
    let mut remaining_chars = budget_chars;
    for skill in skills {
        let line = format!("{}\n", skill.as_listing_line());
        if line.len() > remaining_chars {
            let name_line = format!("- {}\n", skill.name);
            if name_line.len() <= remaining_chars {
                result.push_str(&name_line);
                remaining_chars -= name_line.len();
            } else {
                break;
            }
        } else {
            result.push_str(&line);
            remaining_chars -= line.len();
        }
    }
    result.trim_end().to_string()
}

/// API response wrapper for skill list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SkillsListResponse {
    pub skills: Vec<SkillListItem>,
}

/// Lighter skill record for list endpoints — no content or scripts.
/// Used by both distri-server (OSS) and distri-cloud.
///
/// Note: marketplace fields (`is_public`, `is_system`, `star_count`,
/// `clone_count`, `is_starred`) were removed. Skills are workspace-scoped;
/// public discovery happens through external registries.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SkillListItem {
    pub id: String,
    #[serde(default)]
    pub workspace_slug: String,
    pub name: String,
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub is_owner: bool,
    /// True when the skill belongs to the current workspace
    #[serde(default)]
    pub is_workspace: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Full skill record — content + metadata. Marketplace fields removed; see
/// `SkillListItem` doc comment.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SkillRecord {
    pub id: String,
    /// Workspace slug (cloud: resolved from workspace_id, OSS: "local")
    #[serde(default)]
    pub workspace_slug: String,
    pub name: String,
    /// Full qualified name: "{workspace_slug}/{name}"
    #[serde(default)]
    pub full_name: String,
    pub description: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
    /// Whether the current user owns this skill
    #[serde(default)]
    pub is_owner: bool,
    /// True when the skill belongs to the current workspace
    #[serde(default)]
    pub is_workspace: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Preferred model for skill execution (overrides agent default)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How to deliver skill content: inline (default) or fork (isolated sub-agent)
    #[serde(default)]
    pub context: ContextExecutionType,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[schema(example = json!({"name": "my-skill", "content": "# My Skill\nA helpful utility skill", "description": "A utility skill", "tags": ["utility"]}))]
pub struct NewSkill {
    pub name: String,
    pub description: Option<String>,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub context: ContextExecutionType,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpdateSkill {
    pub name: Option<String>,
    pub description: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextExecutionType>,
}

/// Which slice of skills to return.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SkillScope {
    /// Skills belonging to the current workspace
    #[default]
    Workspace,
    /// Discover external skills (skillsmp.com / GitHub registries)
    Discover,
    /// Workspace + discover combined
    All,
}

/// Filters for listing skills — one struct drives list, search, and pagination.
///
/// `Default` yields `page = 1, per_page = 50` so Rust-side callers like
/// `client.upsert_skill(...)` hit the correct first page. `#[serde(default)]`
/// uses the same values for missing JSON fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFilter {
    /// Which slice of skills to return
    #[serde(default)]
    pub scope: SkillScope,
    /// Full-text search on name/description (empty = no search filter)
    #[serde(default)]
    pub search: Option<String>,
    /// Page number (1-based, default 1)
    #[serde(default = "default_page")]
    pub page: i64,
    /// Items per page (default 50)
    #[serde(default = "default_per_page")]
    pub per_page: i64,
}

impl Default for SkillFilter {
    fn default() -> Self {
        Self {
            scope: SkillScope::default(),
            search: None,
            page: default_page(),
            per_page: default_per_page(),
        }
    }
}

fn default_page() -> i64 {
    1
}
fn default_per_page() -> i64 {
    50
}

/// Paginated skill list response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SkillListResponse {
    pub skills: Vec<SkillListItem>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
}

#[async_trait]
pub trait SkillStore: Send + Sync {
    /// List skills — scope, search, and pagination all via SkillFilter.
    async fn list(&self, filter: SkillFilter) -> anyhow::Result<SkillListResponse>;
    async fn get(&self, id: &str) -> anyhow::Result<Option<SkillRecord>>;
    async fn create(&self, skill: NewSkill) -> anyhow::Result<SkillRecord>;
    async fn update(&self, id: &str, update: UpdateSkill) -> anyhow::Result<SkillRecord>;
    async fn delete(&self, id: &str) -> anyhow::Result<()>;

    /// Create-or-update a skill by name in the caller's current workspace.
    ///
    /// Mirrors `AgentStore::register` semantics: `distri skills push` is an
    /// UPSERT, not a CREATE. Implementations SHOULD do this atomically against
    /// the `(workspace_id, name)` unique constraint (Postgres: `ON CONFLICT
    /// DO UPDATE`). The default impl below is a fall-back for backends that
    /// don't have a native upsert — it performs the list+update-or-create
    /// dance the old client used to do, and inherits its races, so backends
    /// should override it whenever possible.
    async fn upsert_by_name(&self, skill: NewSkill) -> anyhow::Result<SkillRecord> {
        let response = self
            .list(SkillFilter {
                scope: SkillScope::Workspace,
                ..Default::default()
            })
            .await?;
        if let Some(existing) = response.skills.iter().find(|s| s.name == skill.name) {
            return self
                .update(
                    &existing.id,
                    UpdateSkill {
                        name: Some(skill.name),
                        description: skill.description,
                        content: Some(skill.content),
                        tags: Some(skill.tags),
                        model: skill.model,
                        context: Some(skill.context),
                    },
                )
                .await;
        }
        self.create(skill).await
    }
}

// ─── Usage Service ──────────────────────────────────────────────────────────

/// Current usage snapshot for a workspace/user.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UsageSnapshot {
    pub day_tokens: i64,
    pub week_tokens: i64,
    pub month_tokens: i64,
}

/// Configured token limits for a workspace.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UsageLimits {
    pub daily_tokens: Option<i64>,
    pub weekly_tokens: Option<i64>,
    pub monthly_tokens: Option<i64>,
}

/// Result of a rate limit check.
#[derive(Debug, Clone)]
pub enum UsageCheckResult {
    Allowed,
    Denied { reason: String },
}

/// Trait for usage tracking, rate limiting, and workspace limit management.
///
/// OSS: can use a no-op or in-memory implementation.
/// Cloud: backed by Redis + Postgres with caching.
#[async_trait]
pub trait UsageService: Send + Sync {
    /// Check whether a request should be allowed based on all rate limits.
    /// Called by middleware before processing a request.
    /// `is_llm` indicates whether this is an LLM-consuming endpoint.
    /// `auth_source` is "jwt" or "api_key" for per-source analytics.
    async fn check_request(
        &self,
        workspace_id: &str,
        user_id: &str,
        is_llm: bool,
        auth_source: &str,
    ) -> UsageCheckResult;

    /// Record token usage after a completed agent run.
    async fn record_usage(
        &self,
        workspace_id: &str,
        user_id: &str,
        tokens_used: i64,
    ) -> anyhow::Result<()>;

    /// Get current usage snapshot for display.
    async fn get_usage(&self, workspace_id: &str, user_id: &str) -> anyhow::Result<UsageSnapshot>;

    /// Get the configured limits for a workspace.
    async fn get_limits(&self, workspace_id: &str) -> anyhow::Result<UsageLimits>;
}

/// No-op usage service for OSS / development.
/// Always allows requests, never records anything.
#[derive(Debug, Clone)]
pub struct NoOpUsageService;

#[async_trait]
impl UsageService for NoOpUsageService {
    async fn check_request(
        &self,
        _workspace_id: &str,
        _user_id: &str,
        _is_llm: bool,
        _auth_source: &str,
    ) -> UsageCheckResult {
        UsageCheckResult::Allowed
    }

    async fn record_usage(
        &self,
        _workspace_id: &str,
        _user_id: &str,
        _tokens_used: i64,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_usage(
        &self,
        _workspace_id: &str,
        _user_id: &str,
    ) -> anyhow::Result<UsageSnapshot> {
        Ok(UsageSnapshot::default())
    }

    async fn get_limits(&self, _workspace_id: &str) -> anyhow::Result<UsageLimits> {
        Ok(UsageLimits::default())
    }
}

// ========== Connection Store ==========

/// Persistence for connection records (Postgres-backed in cloud).
#[async_trait]
pub trait ConnectionStore: Send + Sync + 'static {
    async fn create(&self, connection: NewConnection) -> anyhow::Result<Connection>;
    async fn get_by_id(&self, id: &str) -> anyhow::Result<Option<Connection>>;
    async fn list_by_workspace(&self, workspace_id: &str) -> anyhow::Result<Vec<Connection>>;
    async fn update_status(&self, id: &str, status: ConnectionStatus) -> anyhow::Result<()>;
    async fn update_skill_id(&self, id: &str, skill_id: uuid::Uuid) -> anyhow::Result<()>;
    /// Rename a connection. Editing the embedded `auth` schema goes through
    /// `update_auth` instead.
    async fn update(&self, id: &str, name: Option<String>) -> anyhow::Result<Connection>;
    /// Replace the `config` JSONB. Used to round-trip workspace-scope
    /// `extra_auth_params` so the Edit dialog can pre-fill them.
    async fn update_config(&self, id: &str, config: serde_json::Value) -> anyhow::Result<()> {
        let _ = (id, config);
        Err(anyhow::anyhow!(
            "update_config not implemented for this ConnectionStore"
        ))
    }
    /// Replace the `auth` JSONB blob. Used by the
    /// `POST /v1/connections/{id}/resync-provider` admin endpoint to re-apply
    /// the catalog-provided `OAuthProviderConfig` onto an existing
    /// connection. `auth` must be a serialized `ConnectionAuth`.
    async fn update_auth(&self, id: &str, auth: serde_json::Value) -> anyhow::Result<()> {
        let _ = (id, auth);
        Err(anyhow::anyhow!(
            "update_auth not implemented for this ConnectionStore"
        ))
    }
    async fn delete(&self, id: &str) -> anyhow::Result<()>;
    /// Look up by `(workspace_id, provider)`. Resolution matches on
    /// `connections.auth->>'provider'` for OAuth.
    async fn get_by_provider(
        &self,
        workspace_id: &str,
        provider: &str,
    ) -> anyhow::Result<Option<Connection>>;
}

/// Token storage for OAuth-auth connections (Redis-backed in cloud).
///
/// **Two key shapes coexist** for the two `AuthScope`s:
///
/// - **Workspace** scope → tokens stored under `connection_id` alone. One
///   slot per connection, shared by every workspace member. `store_token`,
///   `get_token`, `refresh_token` operate on this shape.
/// - **User** scope → sessions stored per `(connection_id, user_id)`. Each
///   end-user authorises themselves; their tokens never bleed into the
///   workspace slot. `get_user_session` / `refresh_user_session` operate
///   on this shape. The cloud implementation persists these via the
///   `ToolAuthStore` (Redis `oauth:session:{provider}:{ws}:{conn}:{user}`),
///   but the API exposes a single `AuthSession` so resolvers don't need
///   to know which underlying store.
#[async_trait]
pub trait ConnectionTokenStore: Send + Sync + 'static {
    async fn store_token(&self, connection_id: &str, token: ConnectionToken) -> anyhow::Result<()>;
    async fn get_token(&self, connection_id: &str) -> anyhow::Result<Option<ConnectionToken>>;
    async fn remove_token(&self, connection_id: &str) -> anyhow::Result<()>;

    /// Attempt to refresh an expired **workspace-scope** OAuth token using
    /// the stored refresh_token. Returns the new token if refresh
    /// succeeds, or None if refresh is not supported or fails. The
    /// implementation should store the refreshed token.
    ///
    /// Cloud implementation uses OAuthHandler.refresh_get_session().
    /// Default: no refresh support (returns None).
    async fn refresh_token(
        &self,
        _connection_id: &str,
        _connection: &Connection,
    ) -> anyhow::Result<Option<ConnectionToken>> {
        Ok(None)
    }

    /// Read the **user-scope** OAuth session for a specific end-user on a
    /// specific Connection. Returns `None` when the user hasn't completed
    /// the configure flow yet. Default impl returns None — only the cloud
    /// `RedisOAuthStore` actually has the underlying `ToolAuthStore` to
    /// dispatch to.
    async fn get_user_session(
        &self,
        _connection: &Connection,
        _user_id: &str,
    ) -> anyhow::Result<Option<crate::auth::AuthSession>> {
        Ok(None)
    }

    /// Refresh an expired **user-scope** OAuth session in place. Returns
    /// the new session on success, `None` if refresh isn't supported / the
    /// refresh_token is missing / the provider rejected the refresh.
    /// Default: no refresh support.
    async fn refresh_user_session(
        &self,
        _connection: &Connection,
        _user_id: &str,
    ) -> anyhow::Result<Option<crate::auth::AuthSession>> {
        Ok(None)
    }

    async fn store_oauth_state(
        &self,
        state_key: &str,
        state: serde_json::Value,
    ) -> anyhow::Result<()>;
    async fn get_oauth_state(&self, state_key: &str) -> anyhow::Result<Option<serde_json::Value>>;
    async fn remove_oauth_state(&self, state_key: &str) -> anyhow::Result<()>;
}

// ========== Note Store ==========

/// Persistence for workspace notes.
///
/// OSS: backed by SQLite via DieselNoteStore.
/// Cloud: backed by Postgres via the existing NoteStore in distri-cloud.
#[async_trait]
pub trait NoteStore: Send + Sync + 'static {
    /// List notes, optionally filtering by tag or full-text search.
    async fn list(
        &self,
        query: &crate::api::notes::ListNotesQuery,
    ) -> anyhow::Result<Vec<crate::api::notes::NoteRecord>>;

    /// Fetch a single note by ID.
    async fn get(&self, id: Uuid) -> anyhow::Result<Option<crate::api::notes::NoteRecord>>;

    /// Create a new note.
    async fn create(
        &self,
        req: crate::api::notes::CreateNoteRequest,
    ) -> anyhow::Result<crate::api::notes::NoteRecord>;

    /// Update an existing note; returns the updated record or `None` if not found.
    async fn update(
        &self,
        id: Uuid,
        req: crate::api::notes::UpdateNoteRequest,
    ) -> anyhow::Result<Option<crate::api::notes::NoteRecord>>;

    /// Delete a note. Returns `true` if the note existed and was deleted.
    async fn delete(&self, id: Uuid) -> anyhow::Result<bool>;

    /// Full-text search on title and content.
    async fn search(&self, query: &str) -> anyhow::Result<Vec<crate::api::notes::NoteRecord>>;
}

// ========== Span Store ==========

/// Query selector for listing spans.
pub enum SpanQuery {
    ByThreadId(String),
    ByTraceId(String),
}

/// Persistence for OTel span records.
///
/// Cloud implements this on top of Postgres; distri-server ships an
/// in-memory implementation that retains spans for the lifetime of the
/// process.
#[async_trait]
pub trait SpanStore: Send + Sync + 'static {
    /// Ingest a batch of spans (idempotent on trace_id + span_id).
    async fn bulk_insert(&self, spans: Vec<crate::api::spans::SpanRecord>)
    -> anyhow::Result<usize>;

    /// Fetch all spans for a trace or thread, ordered by start_time_ns asc.
    async fn list_spans(
        &self,
        workspace_id: &str,
        query: SpanQuery,
    ) -> anyhow::Result<Vec<crate::api::spans::SpanRecord>>;

    /// Aggregate view: one row per trace (root span + per-trace stats).
    async fn list_traces(
        &self,
        workspace_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<crate::api::spans::TraceRecord>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skills_list_response_deserialize_cloud_format() {
        // Tolerates legacy marketplace fields (is_public, star_count, etc.)
        // in the wire format — they're ignored, not deserialized.
        let json = r#"{"skills":[{"id":"abc","workspace_slug":"ws","name":"test","full_name":"ws/test","description":"desc","tags":["t"],"is_public":true,"is_system":false,"is_owner":true,"star_count":0,"clone_count":0,"is_starred":false,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}]}"#;
        let resp: SkillsListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.skills.len(), 1);
        assert_eq!(resp.skills[0].name, "test");
        assert_eq!(resp.skills[0].workspace_slug, "ws");
        assert_eq!(resp.skills[0].full_name, "ws/test");
        assert!(resp.skills[0].is_owner);
    }

    #[test]
    fn test_skills_list_response_deserialize_defaults() {
        let json = r#"{"skills":[{"id":"abc","name":"test","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}]}"#;
        let resp: SkillsListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.skills[0].workspace_slug, "");
        assert_eq!(resp.skills[0].full_name, "");
        assert!(!resp.skills[0].is_owner);
        assert!(!resp.skills[0].is_workspace);
    }

    #[test]
    fn test_skills_list_response_roundtrip() {
        let resp = SkillsListResponse {
            skills: vec![SkillListItem {
                id: "id1".into(),
                workspace_slug: "local".into(),
                name: "my_skill".into(),
                full_name: "local/my_skill".into(),
                description: Some("A skill".into()),
                tags: vec!["tag1".into()],
                is_owner: true,
                is_workspace: true,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: SkillsListResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.skills[0].name, "my_skill");
        assert!(decoded.skills[0].is_workspace);
    }
}
