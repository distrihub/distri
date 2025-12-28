#![allow(dead_code)]
use std::{collections::HashMap, fmt::Display, sync::Arc};

use crate::models::*;
use crate::schema::{
    agent_configs, browser_sessions, external_tool_calls, integrations, memory_entries,
    plugin_catalog, scratchpad_entries, session_entries, task_messages, tasks, threads,
};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use dashmap::DashMap;
use diesel::prelude::*;
use diesel::query_builder::QueryFragment;
use diesel::result::{DatabaseErrorKind, Error as DieselError};
use diesel_async::AsyncConnectionCore;
#[cfg(feature = "sqlite")]
use diesel_async::AsyncMigrationHarness;
use diesel_async::RunQueryDsl;
#[cfg(feature = "sqlite")]
use diesel_async::SimpleAsyncConnection;
use diesel_async::methods::ExecuteDsl;
use diesel_async::pooled_connection::deadpool::{Object, Pool};
use diesel_async::pooled_connection::{AsyncDieselConnectionManager, PoolableConnection};
#[cfg(feature = "sqlite")]
use diesel_async::sync_connection_wrapper::SyncConnectionWrapper;
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use distri_types::auth::{AuthError, AuthSecret, AuthSession, OAuth2State, ToolAuthStore};
use distri_types::configuration::PluginArtifact;
use distri_types::stores::{
    AgentStore, BrowserSessionStore, ExternalToolCallsStore, FilterMessageType, MemoryStore,
    MessageFilter, NewPromptTemplate, NewSecret, PluginCatalogStore, PluginMetadataRecord,
    PromptTemplateRecord, PromptTemplateStore, ScratchpadStore, SecretRecord, SecretStore,
    SessionMemory, SessionStore, TaskStore, ThreadListFilter, ThreadStore, UpdatePromptTemplate,
};
use distri_types::{
    AgentError, AgentEvent, AgentEventType, CreateThreadRequest, Message, ScratchpadEntry, Task,
    TaskEvent, TaskMessage, TaskStatus, Thread, ThreadSummary, ToolResponse, UpdateThreadRequest,
    browser::{BrowserSessionRecord, BrowserSessionState},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value as JsonValue;
use tokio::sync::oneshot;
use tracing::warn;
use uuid::Uuid;

pub const EMBEDDED_MIGRATIONS: EmbeddedMigrations = embed_migrations!("../migrations");

#[cfg(feature = "sqlite")]
pub(crate) type StoreBackend = diesel::sqlite::Sqlite;

#[cfg(all(not(feature = "sqlite"), feature = "postgres"))]
pub(crate) type StoreBackend = diesel::pg::Pg;

pub type DieselConnectionManager<Conn> = AsyncDieselConnectionManager<Conn>;
pub type DieselPool<Conn> = Pool<Conn>;
pub type DieselConn<'a, Conn> = Object<Conn>;

#[cfg(feature = "sqlite")]
pub type SqliteConnectionWrapper = SyncConnectionWrapper<diesel::sqlite::SqliteConnection>;
#[cfg(feature = "sqlite")]
pub type SqliteManager = DieselConnectionManager<SqliteConnectionWrapper>;
#[cfg(feature = "sqlite")]
pub type SqlitePool = DieselPool<SqliteConnectionWrapper>;
#[cfg(feature = "sqlite")]
pub type SqliteConn<'a> = DieselConn<'a, SqliteConnectionWrapper>;
#[cfg(feature = "sqlite")]
pub type SqliteStorePool = DieselStorePool<SqliteConnectionWrapper>;
#[cfg(feature = "sqlite")]
pub type SqliteStoreBuilder = DieselStoreBuilder<SqliteConnectionWrapper>;

#[cfg(all(not(feature = "sqlite"), feature = "postgres"))]
pub type PgManager = DieselConnectionManager<diesel_async::AsyncPgConnection>;
#[cfg(all(not(feature = "sqlite"), feature = "postgres"))]
pub type PgPool = DieselPool<diesel_async::AsyncPgConnection>;
#[cfg(all(not(feature = "sqlite"), feature = "postgres"))]
pub type PgConn<'a> = DieselConn<'a, diesel_async::AsyncPgConnection>;
#[cfg(all(not(feature = "sqlite"), feature = "postgres"))]
pub type PgStorePool = DieselStorePool<diesel_async::AsyncPgConnection>;
#[cfg(all(not(feature = "sqlite"), feature = "postgres"))]
pub type PgStoreBuilder = DieselStoreBuilder<diesel_async::AsyncPgConnection>;

pub trait DieselBackendConnection:
    PoolableConnection + AsyncConnectionCore<Backend = StoreBackend> + Send + 'static
where
    StoreBackend: diesel::backend::DieselReserveSpecialization + diesel::backend::Backend,
{
}

impl<T> DieselBackendConnection for T
where
    T: PoolableConnection + AsyncConnectionCore<Backend = StoreBackend> + Send + 'static,
    StoreBackend: diesel::backend::DieselReserveSpecialization + diesel::backend::Backend,
{
}

fn now_naive() -> NaiveDateTime {
    Utc::now().naive_utc()
}

fn to_naive(dt: DateTime<Utc>) -> NaiveDateTime {
    dt.naive_utc()
}

fn opt_to_naive(dt: Option<DateTime<Utc>>) -> Option<NaiveDateTime> {
    dt.map(|d| d.naive_utc())
}

fn from_naive(dt: NaiveDateTime) -> DateTime<Utc> {
    Utc.from_utc_datetime(&dt)
}

fn task_status_to_str(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running => "running",
        TaskStatus::InputRequired => "input_required",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Canceled => "canceled",
    }
}

fn task_status_from_str(status: &str) -> TaskStatus {
    match status {
        "running" => TaskStatus::Running,
        "input_required" => TaskStatus::InputRequired,
        "completed" => TaskStatus::Completed,
        "failed" => TaskStatus::Failed,
        "canceled" => TaskStatus::Canceled,
        _ => TaskStatus::Pending,
    }
}

fn metadata_to_value_str(metadata: &HashMap<String, JsonValue>) -> Result<String> {
    serde_json::to_string(metadata).context("failed to serialize thread metadata")
}

fn metadata_from_str(value: &str) -> HashMap<String, JsonValue> {
    serde_json::from_str(value).unwrap_or_default()
}

fn to_task(model: TaskModel) -> Task {
    Task {
        id: model.id,
        thread_id: model.thread_id,
        parent_task_id: model.parent_task_id,
        status: task_status_from_str(&model.status),
        created_at: model.created_at,
        updated_at: model.updated_at,
    }
}

fn to_thread(model: ThreadModel) -> Thread {
    Thread {
        id: model.id,
        title: model.title,
        agent_id: model.agent_id.clone(),
        created_at: from_naive(model.created_at),
        updated_at: from_naive(model.updated_at),
        message_count: model.message_count.max(0) as u32,
        last_message: model.last_message,
        metadata: metadata_from_str(&model.metadata),
        attributes: serde_json::from_str(&model.attributes).unwrap_or(serde_json::Value::Null),
        user_id: None,
        external_id: model.external_id,
    }
}

fn to_thread_summary(thread: &Thread) -> ThreadSummary {
    ThreadSummary {
        id: thread.id.clone(),
        agent_id: thread.agent_id.clone(),
        agent_name: String::new(),
        title: thread.title.clone(),
        updated_at: thread.updated_at,
        message_count: thread.message_count,
        last_message: thread.last_message.clone(),
        user_id: thread.user_id.clone(),
        external_id: thread.external_id.clone(),
    }
}

fn to_task_message(model: &TaskMessageModel) -> Result<TaskMessage> {
    match model.kind.as_str() {
        "message" => {
            let message: Message = serde_json::from_str(&model.payload)
                .context("failed to deserialize task message payload")?;
            Ok(TaskMessage::Message(message))
        }
        "event" => {
            let event: TaskEvent = serde_json::from_str(&model.payload)
                .context("failed to deserialize task event payload")?;
            Ok(TaskMessage::Event(event))
        }
        other => Err(anyhow!("unknown task message kind {other}")),
    }
}

fn serialize_agent_event(event: AgentEvent) -> TaskEvent {
    let is_final = matches!(event.event, AgentEventType::RunFinished { .. })
        || matches!(event.event, AgentEventType::RunError { .. });
    TaskEvent {
        event: event.event,
        created_at: Utc::now().timestamp_millis(),
        is_final,
    }
}

fn session_entry_to_value(entry: &SessionEntryModel) -> Option<JsonValue> {
    if let Some(expiry) = entry.expiry {
        if from_naive(expiry) < Utc::now() {
            return None;
        }
    }
    serde_json::from_str(&entry.value).ok()
}

fn message_filter_matches(filter: &MessageFilter, message: &TaskMessage) -> bool {
    match &filter.filter {
        Some(filters) if !filters.is_empty() => match message {
            TaskMessage::Event(_) => filters.contains(&FilterMessageType::Events),
            TaskMessage::Message(_) => {
                filters.contains(&FilterMessageType::Messages)
                    || filters.contains(&FilterMessageType::Artifacts)
            }
        },
        _ => true,
    }
}

fn attributes_match(attributes: &JsonValue, filter: &JsonValue) -> bool {
    match (attributes, filter) {
        (JsonValue::Object(attrs), JsonValue::Object(filter_map)) => filter_map
            .iter()
            .all(|(key, value)| attrs.get(key) == Some(value)),
        _ => true,
    }
}
const GLOBAL_PROVIDER: &str = "__global__";

fn current_timestamp() -> i64 {
    Utc::now().timestamp()
}

fn map_store_error<E: Display>(err: E) -> AuthError {
    AuthError::StoreError(err.to_string())
}

fn serialize_value<T: Serialize>(value: &T) -> Result<String, AuthError> {
    serde_json::to_string(value).map_err(map_store_error)
}

fn deserialize_value<T: DeserializeOwned>(value: &str) -> Result<T, AuthError> {
    serde_json::from_str(value).map_err(map_store_error)
}

fn parse_map<T: DeserializeOwned>(value: &Option<String>) -> Result<HashMap<String, T>, AuthError> {
    match value {
        Some(json) => deserialize_value(json),
        None => Ok(HashMap::new()),
    }
}

fn store_map<T: Serialize>(map: HashMap<String, T>) -> Result<Option<String>, AuthError> {
    if map.is_empty() {
        Ok(None)
    } else {
        serialize_value(&map).map(Some)
    }
}

fn normalize_user_id(user_id: &str) -> Result<String, AuthError> {
    let trimmed = user_id.trim();
    if trimmed.is_empty() {
        Err(AuthError::InvalidConfig(
            "user_id cannot be empty".to_string(),
        ))
    } else {
        Ok(trimmed.to_string())
    }
}

fn normalize_optional_provider(provider: Option<&str>) -> Result<String, AuthError> {
    match provider {
        Some(value) => normalize_provider(value),
        None => Ok(GLOBAL_PROVIDER.to_string()),
    }
}

fn normalize_provider(provider: &str) -> Result<String, AuthError> {
    let trimmed = provider.trim();
    if trimmed.is_empty() {
        Err(AuthError::InvalidConfig(
            "provider/auth_entity cannot be empty".to_string(),
        ))
    } else {
        Ok(trimmed.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredOAuthState {
    redirect_uri: Option<String>,
    user_id: String,
    scopes: Vec<String>,
    metadata: HashMap<String, JsonValue>,
    created_at: i64,
}

/// SQLite-specific pool wrapper
pub struct DieselStorePool<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselPool<Conn>,
}

impl<Conn> DieselStorePool<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselPool<Conn>) -> Self {
        Self { pool }
    }

    pub async fn get(&self) -> Result<DieselConn<'_, Conn>> {
        self.pool
            .get()
            .await
            .context("failed to get connection from pool")
    }

    pub fn clone_pool(&self) -> DieselPool<Conn> {
        self.pool.clone()
    }

    /// Clone the entire store pool (for passing to stores)
    pub fn clone_store_pool(&self) -> Self {
        Self {
            pool: self.pool.clone(),
        }
    }
}

impl<Conn> Clone for DieselStorePool<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
        }
    }
}

#[cfg(feature = "sqlite")]
impl DieselStorePool<SqliteConnectionWrapper> {
    async fn sqlite_pool(database_url: &str, max_connections: u32) -> Result<SqlitePool> {
        tracing::debug!("Creating connection pool with URL: {}", database_url);
        let manager = SqliteManager::new(database_url);
        let pool = SqlitePool::builder(manager)
            .max_size(max_connections as usize)
            .build()
            .context("failed to build SQLite pool")?;

        tracing::debug!("Pool created successfully for: {}", database_url);
        Ok(pool)
    }

    pub async fn new_sqlite(database_url: &str, max_connections: u32) -> Result<Self> {
        let is_in_memory =
            database_url.contains(":memory:") || database_url.contains("mode=memory");

        if is_in_memory {
            // For in-memory databases with shared cache (file:name?mode=memory&cache=shared):
            // 1. Create the pool first
            // 2. Get a connection from the pool
            // 3. Run migrations on that connection's inner sync connection
            // 4. Return connection to pool - all future pool connections share the same DB
            // See: https://github.com/weiznich/diesel_async/issues/213
            tracing::debug!(
                "Creating ephemeral in-memory SQLite with shared cache: {}",
                database_url
            );

            // Create pool first

            let pool = Self::sqlite_pool(database_url, max_connections).await?;

            // Run migrations using AsyncMigrationHarness
            // The harness provides sync methods that work directly with the connection
            tracing::info!("Running migrations on in-memory database");

            let mut harness = AsyncMigrationHarness::new(pool.get().await?);
            tokio::task::spawn_blocking(move || -> Result<()> {
                diesel_migrations::MigrationHarness::run_pending_migrations(
                    &mut harness,
                    EMBEDDED_MIGRATIONS,
                )
                .map_err(|err| anyhow!("failed to run diesel migrations: {err}"))?;
                Ok(())
            })
            .await
            .map_err(|err| anyhow!("migration thread panicked: {err}"))??;
            // TODO: this will not work per request
            // harness
            //     .run_pending_migrations(EMBEDDED_MIGRATIONS)
            //     .map_err(|err| anyhow!("failed to run diesel migrations: {err}"))?;

            tracing::info!("In-memory database initialized with migrations ✅");

            // Connection returns to pool, database stays alive due to shared cache
            tracing::debug!("In-memory pool ready ✅");

            Ok(Self { pool })
        } else {
            // For file-based databases, use connection pool normally
            run_migrations(database_url).await?;

            let pool = Self::sqlite_pool(database_url, max_connections).await?;
            Ok(Self { pool })
        }
    }
}

#[cfg(all(not(feature = "sqlite"), feature = "postgres"))]
impl DieselStorePool<diesel_async::AsyncPgConnection> {
    pub async fn postgres_pool(database_url: &str, max_connections: u32) -> Result<PgPool> {
        let manager = PgManager::new(database_url);
        let pool = PgPool::builder(manager)
            .max_size(max_connections as usize)
            .build()
            .context("failed to build Postgres pool")?;
        Ok(pool)
    }

    pub async fn new_postgres(database_url: &str, max_connections: u32) -> Result<Self> {
        let pool = Self::postgres_pool(database_url, max_connections).await?;
        Ok(Self { pool })
    }
}

#[cfg(feature = "sqlite")]
async fn run_migrations(database_url: &str) -> Result<()> {
    tracing::debug!("Running migrations for database: {}", database_url);

    // Create a temporary connection pool to run migrations
    let manager = SqliteManager::new(database_url);
    let pool = SqlitePool::builder(manager)
        .max_size(1)
        .build()
        .context("failed to create migration pool")?;

    let mut conn = pool
        .get()
        .await
        .context("failed to get migration connection")?;

    // Enable WAL mode for file-based databases (done via pragma)
    conn.batch_execute("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
        .await
        .context("failed to configure SQLite")?;

    // Run migrations using AsyncMigrationHarness inside spawn_blocking
    let mut harness = diesel_async::AsyncMigrationHarness::new(conn);
    tokio::task::spawn_blocking(move || -> Result<()> {
        diesel_migrations::MigrationHarness::run_pending_migrations(
            &mut harness,
            EMBEDDED_MIGRATIONS,
        )
        .map_err(|err| anyhow!("failed to run diesel migrations: {err}"))?;
        Ok(())
    })
    .await
    .map_err(|err| anyhow!("migration thread panicked: {err}"))??;

    tracing::debug!("Migrations completed successfully for: {}", database_url);
    Ok(())
}

#[derive(Clone)]
pub struct DieselAgentStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselStorePool<Conn>,
}

impl<Conn> DieselAgentStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselStorePool<Conn>) -> Self {
        Self { pool }
    }

    async fn conn(&self) -> Result<DieselConn<'_, Conn>> {
        self.pool
            .get()
            .await
            .context("failed to acquire diesel connection")
    }
}

#[async_trait]
impl<Conn> AgentStore for DieselAgentStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    async fn list(
        &self,
        cursor: Option<String>,
        limit: Option<usize>,
    ) -> (
        Vec<distri_types::configuration::AgentConfig>,
        Option<String>,
    ) {
        let mut connection = match self.conn().await {
            Ok(conn) => conn,
            Err(err) => {
                warn!("failed to list agents via diesel store: {err:?}");
                return (vec![], cursor);
            }
        };

        let mut query = agent_configs::table.into_boxed();
        if let Some(ref cursor_value) = cursor {
            query = query.filter(agent_configs::name.gt(cursor_value));
        }

        let fetch_limit = limit.unwrap_or(100) as i64;
        let rows = match query
            .order(agent_configs::name.asc())
            .limit(fetch_limit + 1)
            .load::<AgentConfigModel>(&mut connection)
            .await
        {
            Ok(rows) => rows,
            Err(err) => {
                warn!("failed to load agent configs: {err:?}");
                return (vec![], None);
            }
        };

        let mut configs = Vec::new();
        let mut next_cursor = None;

        for (index, row) in rows.into_iter().enumerate() {
            if index as i64 == fetch_limit {
                next_cursor = Some(row.name.clone());
                break;
            }

            match serde_json::from_str(&row.config) {
                Ok(config) => configs.push(config),
                Err(err) => warn!("failed to deserialize agent config: {err:?}"),
            }
        }

        (configs, next_cursor)
    }

    async fn get(&self, name: &str) -> Option<distri_types::configuration::AgentConfig> {
        let mut connection = self.conn().await.ok()?;
        agent_configs::table
            .find(name)
            .first::<AgentConfigModel>(&mut connection)
            .await
            .ok()
            .and_then(|row| serde_json::from_str(&row.config).ok())
    }

    async fn register(&self, config: distri_types::configuration::AgentConfig) -> Result<()> {
        let mut connection = self.conn().await?;
        let name = config.get_name().to_string();
        let serialized =
            serde_json::to_string(&config).context("failed to serialize agent config")?;
        let timestamp = now_naive();

        let insert = NewAgentConfigModel {
            name: &name,
            config: &serialized,
            created_at: timestamp,
            updated_at: timestamp,
        };

        let changes = AgentConfigChangeset {
            config: &serialized,
            updated_at: timestamp,
        };

        diesel::insert_into(agent_configs::table)
            .values(&insert)
            .on_conflict(agent_configs::name)
            .do_update()
            .set(&changes)
            .execute(&mut connection)
            .await
            .context("failed to upsert agent config")?;

        Ok(())
    }

    async fn update(&self, config: distri_types::configuration::AgentConfig) -> Result<()> {
        self.register(config).await
    }

    async fn clear(&self) -> Result<()> {
        let mut connection = self.conn().await?;
        diesel::delete(agent_configs::table)
            .execute(&mut connection)
            .await
            .context("failed to clear agent configs")?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct DieselThreadStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselStorePool<Conn>,
}

impl<Conn> DieselThreadStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselStorePool<Conn>) -> Self {
        Self { pool }
    }

    async fn conn(&self) -> Result<DieselConn<'_, Conn>> {
        self.pool
            .get()
            .await
            .context("failed to acquire diesel connection")
    }

    async fn fetch_thread(&self, thread_id: &str) -> Result<Option<Thread>> {
        let mut connection = self
            .conn()
            .await
            .context("failed to get connection for thread fetch")?;
        let row = threads::table
            .find(thread_id)
            .first::<ThreadModel>(&mut connection)
            .await
            .optional()
            .context(format!(
                "failed to query thread table for thread_id: {}",
                thread_id
            ))?;
        Ok(row.map(to_thread))
    }
}

impl<Conn> std::fmt::Debug for DieselThreadStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DieselThreadStore").finish()
    }
}

#[async_trait]
impl<Conn> ThreadStore for DieselThreadStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn create_thread(&self, request: CreateThreadRequest) -> Result<Thread> {
        let mut thread = Thread::new(
            request.agent_id,
            request.title,
            request.thread_id,
            None,
            request.external_id,
        );
        if let Some(attributes) = request.attributes {
            thread.attributes = attributes;
        }

        let metadata_value = metadata_to_value_str(&thread.metadata)?;
        let created_at = to_naive(thread.created_at);
        let updated_at = to_naive(thread.updated_at);

        let new_model = NewThreadModel {
            id: &thread.id,
            agent_id: &thread.agent_id,
            title: &thread.title,
            created_at,
            updated_at,
            message_count: thread.message_count as i32,
            last_message: thread.last_message.as_deref(),
            metadata: &metadata_value,
            attributes: &thread.attributes.to_string(),
            external_id: thread.external_id.as_deref(),
        };

        let mut connection = self
            .conn()
            .await
            .context("failed to get connection for create_thread")?;

        #[cfg(feature = "sqlite")]
        {
            // Debug: Check what tables are visible to this connection
            use diesel::QueryableByName;
            #[derive(QueryableByName)]
            struct TableName {
                #[diesel(sql_type = diesel::sql_types::Text)]
                name: String,
            }

            let tables_result = diesel::dsl::sql_query(
                "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
            )
            .load::<TableName>(&mut connection)
            .await;

            match tables_result {
                Ok(rows) => {
                    let names: Vec<String> = rows.into_iter().map(|t| t.name).collect();
                    tracing::debug!("Pool connection sees tables: {:?}", names);
                }
                Err(e) => tracing::error!("Pool connection failed to list tables: {}", e),
            }
        }

        tracing::debug!(
            "Attempting to insert thread {} into threads table",
            thread.id
        );
        diesel::insert_into(threads::table)
            .values(&new_model)
            .execute(&mut connection)
            .await
            .map_err(|e| {
                tracing::error!("Failed to insert thread {}: {}", thread.id, e);
                anyhow!("failed to insert thread (thread_id: {}): {}", thread.id, e)
            })?;

        Ok(thread)
    }

    async fn get_thread(&self, thread_id: &str) -> Result<Option<Thread>> {
        self.fetch_thread(thread_id).await
    }

    async fn update_thread(&self, thread_id: &str, request: UpdateThreadRequest) -> Result<Thread> {
        let mut connection = self.conn().await?;
        let record = threads::table
            .find(thread_id)
            .first::<ThreadModel>(&mut connection)
            .await
            .context("thread not found")?;

        let mut thread = to_thread(record);

        if let Some(title) = request.title {
            thread.title = title;
        }
        if let Some(metadata) = request.metadata {
            thread.metadata.extend(metadata);
        }
        if let Some(attributes) = request.attributes {
            thread.attributes = attributes;
        }

        thread.updated_at = Utc::now();

        let metadata_value = metadata_to_value_str(&thread.metadata)?;
        let attr_str = thread.attributes.to_string();

        let changeset = ThreadChangeset {
            title: Some(&thread.title),
            updated_at: to_naive(thread.updated_at),
            message_count: Some(thread.message_count as i32),
            last_message: Some(thread.last_message.as_deref()),
            metadata: Some(&metadata_value),
            attributes: Some(&attr_str),
            external_id: None,
        };

        diesel::update(threads::table.find(thread_id))
            .set(&changeset)
            .execute(&mut connection)
            .await
            .context("failed to update thread")?;

        Ok(thread)
    }

    async fn delete_thread(&self, thread_id: &str) -> Result<()> {
        let mut connection = self.conn().await?;
        diesel::delete(threads::table.filter(threads::id.eq(thread_id)))
            .execute(&mut connection)
            .await
            .context("failed to delete thread")?;
        Ok(())
    }

    async fn list_threads(
        &self,
        filter: &ThreadListFilter,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<ThreadSummary>> {
        let mut connection = self.conn().await?;
        let mut query = threads::table.into_boxed();

        // Local store is single-tenant, so we ignore user_id filter
        // Filter by agent_id if provided
        if let Some(agent) = &filter.agent_id {
            query = query.filter(threads::agent_id.eq(agent.as_str()));
        }

        // Filter by external_id if provided
        if let Some(ext_id) = &filter.external_id {
            query = query.filter(threads::external_id.eq(ext_id.as_str()));
        }

        let rows = query
            .order(threads::updated_at.desc())
            .offset(offset.unwrap_or(0) as i64)
            .limit(limit.unwrap_or(50) as i64)
            .load::<ThreadModel>(&mut connection)
            .await?;

        let mut summaries = Vec::new();

        for row in rows {
            let thread = to_thread(row);
            if filter
                .attributes
                .as_ref()
                .map_or(true, |f| attributes_match(&thread.attributes, f))
            {
                summaries.push(to_thread_summary(&thread));
            }
        }

        Ok(summaries)
    }

    async fn update_thread_with_message(&self, thread_id: &str, message: &str) -> Result<()> {
        let mut connection = self.conn().await?;
        let record = threads::table
            .find(thread_id)
            .first::<ThreadModel>(&mut connection)
            .await
            .context("thread not found")?;

        let mut thread = to_thread(record);
        thread.update_with_message(message);

        let metadata_value = metadata_to_value_str(&thread.metadata)?;
        let attr_str = thread.attributes.to_string();

        let changeset = ThreadChangeset {
            title: Some(&thread.title),
            updated_at: to_naive(thread.updated_at),
            message_count: Some(thread.message_count as i32),
            last_message: Some(thread.last_message.as_deref()),
            metadata: Some(&metadata_value),
            attributes: Some(&attr_str),
            external_id: None,
        };

        diesel::update(threads::table.find(thread_id))
            .set(&changeset)
            .execute(&mut connection)
            .await
            .context("failed to update thread with message")?;

        Ok(())
    }

    async fn get_home_stats(&self) -> Result<distri_types::stores::HomeStats> {
        let mut connection = self.conn().await?;

        // Count total agents
        let total_agents = agent_configs::table
            .count()
            .get_result::<i64>(&mut connection)
            .await
            .context("Failed to count agents")?;

        // Count total threads
        let total_threads = threads::table
            .count()
            .get_result::<i64>(&mut connection)
            .await
            .context("Failed to count threads")?;

        // Sum all message counts from threads
        let total_messages: Option<i64> = threads::table
            .select(diesel::dsl::sum(threads::message_count))
            .first(&mut connection)
            .await
            .context("Failed to sum message counts")?;

        // Get latest 5 threads
        let latest_thread_rows: Vec<(String, String, String, NaiveDateTime)> = threads::table
            .select((
                threads::id,
                threads::title,
                threads::agent_id,
                threads::updated_at,
            ))
            .order(threads::updated_at.desc())
            .limit(5)
            .load(&mut connection)
            .await
            .context("Failed to load latest threads")?;

        let latest_threads: Vec<distri_types::stores::LatestThreadInfo> = latest_thread_rows
            .into_iter()
            .map(
                |(id, title, aid, updated_at)| distri_types::stores::LatestThreadInfo {
                    id,
                    title,
                    agent_id: aid.clone(),
                    agent_name: aid,
                    updated_at: chrono::DateTime::from_naive_utc_and_offset(
                        updated_at,
                        chrono::Utc,
                    ),
                },
            )
            .collect();

        // Get most active agent (agent with most threads)
        #[derive(QueryableByName)]
        struct AgentThreadCount {
            #[diesel(sql_type = diesel::sql_types::Text)]
            agent_id: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            agent_name: String,
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            thread_count: i64,
        }

        let most_active: Option<AgentThreadCount> = diesel::sql_query(
            "SELECT t.agent_id, t.agent_id as agent_name, COUNT(*) as thread_count
             FROM threads t
             LEFT JOIN agent_configs a ON t.agent_id = a.name
             GROUP BY t.agent_id, a.name
             ORDER BY thread_count DESC
             LIMIT 1",
        )
        .get_result(&mut connection)
        .await
        .optional()
        .context("Failed to find most active agent")?;

        let most_active_agent = most_active.map(|a| distri_types::stores::MostActiveAgent {
            id: a.agent_id,
            name: a.agent_name,
            thread_count: a.thread_count,
        });

        // Calculate average run time from tasks
        #[derive(QueryableByName)]
        struct AvgResult {
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
            avg_duration: Option<f64>,
        }

        let avg_result: Option<AvgResult> = diesel::sql_query(
            "SELECT AVG(updated_at - created_at) as avg_duration FROM tasks WHERE status = 'completed'",
        )
        .get_result(&mut connection)
        .await
        .optional()
        .context("Failed to calculate average run time")?;

        let avg_run_time_ms = avg_result.and_then(|r| r.avg_duration);

        Ok(distri_types::stores::HomeStats {
            total_agents,
            total_threads,
            total_messages: total_messages.unwrap_or(0),
            avg_run_time_ms,
            // Local is single-tenant, so owned = accessible = total
            total_owned_agents: Some(total_agents),
            total_accessible_agents: Some(total_agents),
            most_active_agent,
            latest_threads: Some(latest_threads),
        })
    }
}

#[derive(Clone)]
pub struct DieselTaskStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselStorePool<Conn>,
}

impl<Conn> DieselTaskStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselStorePool<Conn>) -> Self {
        Self { pool }
    }

    async fn conn(&self) -> Result<DieselConn<'_, Conn>> {
        self.pool
            .get()
            .await
            .context("failed to acquire diesel connection")
    }
}

#[async_trait]
impl<Conn> TaskStore for DieselTaskStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    async fn create_task(
        &self,
        context_id: &str,
        task_id: Option<&str>,
        task_status: Option<TaskStatus>,
    ) -> Result<Task> {
        let mut connection = self.conn().await?;
        let task_id = task_id
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let status = task_status.unwrap_or(TaskStatus::Pending);
        let now = Utc::now().timestamp_millis();

        let new_task = NewTaskModel {
            id: &task_id,
            thread_id: context_id,
            parent_task_id: None,
            status: task_status_to_str(&status),
            created_at: now,
            updated_at: now,
        };

        diesel::insert_into(tasks::table)
            .values(&new_task)
            .execute(&mut connection)
            .await
            .context("failed to insert task")?;

        Ok(Task {
            id: task_id,
            thread_id: context_id.to_string(),
            parent_task_id: None,
            status,
            created_at: now,
            updated_at: now,
        })
    }

    async fn get_task(&self, task_id: &str) -> Result<Option<Task>> {
        let mut connection = self.conn().await?;
        let row = tasks::table
            .find(task_id)
            .first::<TaskModel>(&mut connection)
            .await
            .optional()
            .context("failed to load task")?;
        Ok(row.map(to_task))
    }

    async fn add_event_to_task(&self, task_id: &str, event: AgentEvent) -> Result<()> {
        let mut connection = self.conn().await?;
        let task_event = serialize_agent_event(event);
        let payload =
            serde_json::to_string(&task_event).context("failed to serialize task event")?;

        let new_message = NewTaskMessageModel {
            task_id,
            kind: "event",
            payload: &payload,
            created_at: task_event.created_at,
        };

        diesel::insert_into(task_messages::table)
            .values(&new_message)
            .execute(&mut connection)
            .await
            .context("failed to insert task event")?;

        Ok(())
    }

    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> Result<()> {
        let mut connection = self.conn().await?;
        let changeset = TaskStatusChangeset {
            status: task_status_to_str(&status),
            updated_at: Utc::now().timestamp_millis(),
        };

        diesel::update(tasks::table.find(task_id))
            .set(&changeset)
            .execute(&mut connection)
            .await
            .context("failed to update task status")?;

        Ok(())
    }

    async fn update_parent_task(&self, task_id: &str, parent_task_id: Option<&str>) -> Result<()> {
        let mut connection = self.conn().await?;
        diesel::update(tasks::table.find(task_id))
            .set((
                tasks::parent_task_id.eq(parent_task_id),
                tasks::updated_at.eq(Utc::now().timestamp_millis()),
            ))
            .execute(&mut connection)
            .await
            .context("failed to update task parent")?;
        Ok(())
    }

    async fn cancel_task(&self, task_id: &str) -> Result<Task> {
        self.update_task_status(task_id, TaskStatus::Canceled)
            .await?;
        self.get_task(task_id)
            .await?
            .ok_or_else(|| anyhow!("task not found after cancel"))
    }

    async fn add_message_to_task(&self, task_id: &str, message: &Message) -> Result<()> {
        let mut connection = self.conn().await?;
        let payload = serde_json::to_string(message).context("failed to serialize task message")?;

        let new_message = NewTaskMessageModel {
            task_id,
            kind: "message",
            payload: &payload,
            created_at: message.created_at,
        };

        diesel::insert_into(task_messages::table)
            .values(&new_message)
            .execute(&mut connection)
            .await
            .context("failed to insert task message")?;

        Ok(())
    }

    async fn list_tasks(&self, context_id: Option<&str>) -> Result<Vec<Task>> {
        let mut connection = self.conn().await?;
        let mut query = tasks::table.into_boxed();

        if let Some(thread_id) = context_id {
            query = query.filter(tasks::thread_id.eq(thread_id));
        }

        let rows = query
            .order(tasks::created_at.asc())
            .load::<TaskModel>(&mut connection)
            .await
            .context("failed to list tasks")?;

        Ok(rows.into_iter().map(to_task).collect())
    }

    async fn get_history(
        &self,
        thread_id: &str,
        filter: Option<MessageFilter>,
    ) -> Result<Vec<(Task, Vec<TaskMessage>)>> {
        let mut connection = self.conn().await?;
        let task_rows = tasks::table
            .filter(tasks::thread_id.eq(thread_id))
            .order(tasks::created_at.asc())
            .load::<TaskModel>(&mut connection)
            .await
            .context("failed to load tasks for history")?;

        if task_rows.is_empty() {
            return Ok(vec![]);
        }

        let task_ids: Vec<String> = task_rows.iter().map(|task| task.id.clone()).collect();

        let message_rows = task_messages::table
            .filter(task_messages::task_id.eq_any(&task_ids))
            .order(task_messages::created_at.asc())
            .load::<TaskMessageModel>(&mut connection)
            .await
            .context("failed to load task messages")?;

        let mut grouped: HashMap<String, Vec<TaskMessage>> = HashMap::new();
        for row in &message_rows {
            match to_task_message(row) {
                Ok(message) => grouped
                    .entry(row.task_id.clone())
                    .or_default()
                    .push(message),
                Err(err) => warn!("failed to deserialize task message: {err:?}"),
            }
        }

        let filter = filter.unwrap_or(MessageFilter {
            filter: None,
            limit: None,
            offset: None,
        });
        let mut history = Vec::new();

        for task_row in task_rows {
            let task = to_task(task_row.clone());
            let messages = grouped
                .get(&task.id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|msg| message_filter_matches(&filter, msg))
                .collect::<Vec<_>>();
            history.push((task, messages));
        }

        if let Some(offset) = filter.offset {
            history = history.into_iter().skip(offset).collect();
        }

        if let Some(limit) = filter.limit {
            history.truncate(limit);
        }

        Ok(history)
    }
}

#[derive(Clone)]
pub struct DieselSessionStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselStorePool<Conn>,
}

impl<Conn> DieselSessionStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselStorePool<Conn>) -> Self {
        Self { pool }
    }

    async fn conn(&self) -> Result<DieselConn<'_, Conn>> {
        self.pool
            .get()
            .await
            .context("failed to acquire diesel connection")
    }
}

impl<Conn> std::fmt::Debug for DieselSessionStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DieselSessionStore").finish()
    }
}

#[async_trait]
impl<Conn> SessionStore for DieselSessionStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    async fn clear_session(&self, namespace: &str) -> Result<()> {
        let mut connection = self.conn().await?;
        diesel::delete(session_entries::table.filter(session_entries::thread_id.eq(namespace)))
            .execute(&mut connection)
            .await
            .context("failed to clear session namespace")?;
        Ok(())
    }

    async fn set_value(&self, namespace: &str, key: &str, value: &JsonValue) -> Result<()> {
        self.set_value_with_expiry(namespace, key, value, None)
            .await
    }

    async fn set_value_with_expiry(
        &self,
        namespace: &str,
        key: &str,
        value: &JsonValue,
        expiry: Option<DateTime<Utc>>,
    ) -> Result<()> {
        let mut connection = self.conn().await?;
        let now = now_naive();
        let value_str =
            serde_json::to_string(value).context("failed to serialize session value")?;

        let insertion = NewSessionEntryModel {
            thread_id: namespace,
            key,
            value: &value_str,
            expiry: opt_to_naive(expiry),
            created_at: now,
            updated_at: now,
        };

        let changeset = SessionEntryChangeset {
            value: &value_str,
            expiry: opt_to_naive(expiry),
            updated_at: now,
        };

        diesel::insert_into(session_entries::table)
            .values(&insertion)
            .on_conflict((session_entries::thread_id, session_entries::key))
            .do_update()
            .set(&changeset)
            .execute(&mut connection)
            .await
            .context("failed to set session value")?;

        Ok(())
    }

    async fn get_value(&self, namespace: &str, key: &str) -> Result<Option<JsonValue>> {
        let mut connection = self.conn().await?;
        let row = session_entries::table
            .filter(session_entries::thread_id.eq(namespace))
            .filter(session_entries::key.eq(key))
            .first::<SessionEntryModel>(&mut connection)
            .await
            .optional()
            .context("failed to fetch session entry")?;

        Ok(row.as_ref().and_then(session_entry_to_value))
    }

    async fn delete_value(&self, namespace: &str, key: &str) -> Result<()> {
        let mut connection = self.conn().await?;
        diesel::delete(
            session_entries::table
                .filter(session_entries::thread_id.eq(namespace))
                .filter(session_entries::key.eq(key)),
        )
        .execute(&mut connection)
        .await
        .context("failed to delete session entry")?;
        Ok(())
    }

    async fn get_all_values(&self, namespace: &str) -> Result<HashMap<String, JsonValue>> {
        let mut connection = self.conn().await?;
        let entries = session_entries::table
            .filter(session_entries::thread_id.eq(namespace))
            .load::<SessionEntryModel>(&mut connection)
            .await
            .context("failed to fetch session entries")?;

        let mut map = HashMap::new();

        for entry in entries {
            if let Some(value) = session_entry_to_value(&entry) {
                map.insert(entry.key.clone(), value);
            } else {
                diesel::delete(
                    session_entries::table
                        .filter(session_entries::thread_id.eq(namespace))
                        .filter(session_entries::key.eq(&entry.key)),
                )
                .execute(&mut connection)
                .await
                .ok();
            }
        }

        Ok(map)
    }
}

#[derive(Clone)]
pub struct DieselMemoryStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselStorePool<Conn>,
}

impl<Conn> DieselMemoryStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselStorePool<Conn>) -> Self {
        Self { pool }
    }

    async fn conn(&self) -> Result<DieselConn<'_, Conn>> {
        self.pool
            .get()
            .await
            .context("failed to acquire diesel connection")
    }
}

#[async_trait]
impl<Conn> MemoryStore for DieselMemoryStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    async fn store_memory(&self, user_id: &str, session_memory: SessionMemory) -> Result<()> {
        let mut connection = self.conn().await?;
        let content = format!(
            "Agent: {} | Session: {} ({})\nSummary: {}\nInsights: {}\nFacts: {}",
            session_memory.agent_id,
            session_memory.thread_id,
            session_memory.timestamp.format("%Y-%m-%d %H:%M:%S"),
            session_memory.session_summary,
            session_memory.key_insights.join("; "),
            session_memory.important_facts.join("; "),
        );

        let new_entry = NewMemoryEntryModel {
            user_id,
            content: &content,
            created_at: now_naive(),
        };

        diesel::insert_into(memory_entries::table)
            .values(&new_entry)
            .execute(&mut connection)
            .await
            .context("failed to insert memory entry")?;

        Ok(())
    }

    async fn search_memories(
        &self,
        user_id: &str,
        query: &str,
        limit: Option<usize>,
    ) -> Result<Vec<String>> {
        let mut connection = self.conn().await?;
        let rows = memory_entries::table
            .filter(memory_entries::user_id.eq(user_id))
            .order(memory_entries::created_at.desc())
            .load::<MemoryEntryModel>(&mut connection)
            .await
            .context("failed to load memory entries")?;

        let mut results = Vec::new();
        let query_lower = query.to_lowercase();

        for row in rows {
            let content = row.content;
            if content.to_lowercase().contains(&query_lower) {
                results.push(content);

                if limit.map_or(false, |limit| results.len() >= limit) {
                    break;
                }
            }
        }

        Ok(results)
    }

    async fn get_user_memories(&self, user_id: &str) -> Result<Vec<String>> {
        let mut connection = self.conn().await?;
        let rows = memory_entries::table
            .filter(memory_entries::user_id.eq(user_id))
            .order(memory_entries::created_at.desc())
            .load::<MemoryEntryModel>(&mut connection)
            .await
            .context("failed to load memories")?;

        Ok(rows.into_iter().map(|row| row.content).collect())
    }

    async fn clear_user_memories(&self, user_id: &str) -> Result<()> {
        let mut connection = self.conn().await?;
        diesel::delete(memory_entries::table.filter(memory_entries::user_id.eq(user_id)))
            .execute(&mut connection)
            .await
            .context("failed to clear user memories")?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct DieselScratchpadStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselStorePool<Conn>,
}

impl<Conn> DieselScratchpadStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselStorePool<Conn>) -> Self {
        Self { pool }
    }

    async fn conn(&self) -> Result<DieselConn<'_, Conn>> {
        self.pool
            .get()
            .await
            .context("failed to acquire diesel connection")
    }
}

impl<Conn> std::fmt::Debug for DieselScratchpadStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DieselScratchpadStore").finish()
    }
}

#[async_trait]
impl<Conn> ScratchpadStore for DieselScratchpadStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    async fn add_entry(&self, thread_id: &str, entry: ScratchpadEntry) -> Result<(), AgentError> {
        let mut connection = self
            .conn()
            .await
            .map_err(|err| AgentError::Execution(err.to_string()))?;

        let payload =
            serde_json::to_string(&entry).map_err(|err| AgentError::Execution(err.to_string()))?;

        let new_entry = NewScratchpadEntryModel {
            thread_id,
            task_id: &entry.task_id,
            parent_task_id: entry.parent_task_id.as_deref(),
            entry: &payload,
            entry_type: entry.entry_kind.as_deref(),
            timestamp: entry.timestamp,
            created_at: now_naive(),
        };

        diesel::insert_into(scratchpad_entries::table)
            .values(&new_entry)
            .execute(&mut connection)
            .await
            .map_err(|err| AgentError::Execution(err.to_string()))?;

        Ok(())
    }

    async fn clear_entries(&self, thread_id: &str) -> Result<(), AgentError> {
        let mut connection = self
            .conn()
            .await
            .map_err(|err| AgentError::Execution(err.to_string()))?;

        diesel::delete(
            scratchpad_entries::table.filter(scratchpad_entries::thread_id.eq(thread_id)),
        )
        .execute(&mut connection)
        .await
        .map_err(|err| AgentError::Execution(err.to_string()))?;
        Ok(())
    }
    async fn get_all_entries(
        &self,
        thread_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ScratchpadEntry>, AgentError> {
        let mut connection = self
            .conn()
            .await
            .map_err(|err| AgentError::Execution(err.to_string()))?;

        let mut query = scratchpad_entries::table
            .filter(scratchpad_entries::thread_id.eq(thread_id))
            // Exclude entries that belong to sub-tasks (parent_task_id set)
            .filter(scratchpad_entries::parent_task_id.is_null())
            .order(scratchpad_entries::created_at.desc())
            .into_boxed();

        if let Some(limit) = limit {
            query = query.limit(limit as i64);
        }

        let rows = query
            .load::<ScratchpadEntryModel>(&mut connection)
            .await
            .map_err(|err| AgentError::Execution(err.to_string()))?;

        let mut entries = Vec::new();
        for row in rows {
            match serde_json::from_str::<ScratchpadEntry>(&row.entry) {
                Ok(mut entry) => {
                    if entry.entry_kind.is_none() {
                        entry.entry_kind = row.entry_type.clone();
                    }
                    if entry.parent_task_id.is_none() {
                        entry.parent_task_id = row.parent_task_id.clone();
                    }
                    entries.push(entry)
                }
                Err(err) => warn!("failed to deserialize scratchpad entry: {err:?}"),
            }
        }

        entries.reverse();
        Ok(entries)
    }
    async fn get_entries(
        &self,
        thread_id: &str,
        task_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ScratchpadEntry>, AgentError> {
        let mut connection = self
            .conn()
            .await
            .map_err(|err| AgentError::Execution(err.to_string()))?;

        let mut query = scratchpad_entries::table
            .filter(scratchpad_entries::thread_id.eq(thread_id))
            .filter(scratchpad_entries::task_id.eq(task_id))
            .order(scratchpad_entries::created_at.desc())
            .into_boxed();

        if let Some(limit) = limit {
            query = query.limit(limit as i64);
        }

        let rows = query
            .load::<ScratchpadEntryModel>(&mut connection)
            .await
            .map_err(|err| AgentError::Execution(err.to_string()))?;

        let mut entries = Vec::new();
        for row in rows {
            match serde_json::from_str::<ScratchpadEntry>(&row.entry) {
                Ok(mut entry) => {
                    if entry.entry_kind.is_none() {
                        entry.entry_kind = row.entry_type.clone();
                    }
                    if entry.parent_task_id.is_none() {
                        entry.parent_task_id = row.parent_task_id.clone();
                    }
                    entries.push(entry)
                }
                Err(err) => warn!("failed to deserialize scratchpad entry: {err:?}"),
            }
        }

        entries.reverse();
        Ok(entries)
    }
}

#[derive(Clone)]
pub struct DieselBrowserSessionStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselStorePool<Conn>,
}

impl<Conn> DieselBrowserSessionStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselStorePool<Conn>) -> Self {
        Self { pool }
    }

    async fn conn(&self) -> Result<DieselConn<'_, Conn>> {
        self.pool
            .get()
            .await
            .context("failed to acquire diesel connection")
    }
}

impl<Conn> std::fmt::Debug for DieselBrowserSessionStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DieselBrowserSessionStore").finish()
    }
}

#[async_trait]
impl<Conn> BrowserSessionStore for DieselBrowserSessionStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    async fn save_session(&self, record: BrowserSessionRecord) -> anyhow::Result<()> {
        let mut connection = self.conn().await?;
        let state_json = serde_json::to_string(&record.state)
            .context("failed to serialize browser session state")?;
        let timestamp = to_naive(record.updated_at);

        let insert = NewBrowserSessionModel {
            user_id: &record.user_id,
            state: &state_json,
            created_at: timestamp,
            updated_at: timestamp,
        };

        let changeset = BrowserSessionChangeset {
            state: &state_json,
            updated_at: timestamp,
        };

        diesel::insert_into(browser_sessions::table)
            .values(&insert)
            .on_conflict(browser_sessions::user_id)
            .do_update()
            .set(&changeset)
            .execute(&mut connection)
            .await
            .context("failed to upsert browser session state")?;

        Ok(())
    }

    async fn get_session(&self, user_id: &str) -> anyhow::Result<Option<BrowserSessionRecord>> {
        let mut connection = self.conn().await?;
        match browser_sessions::table
            .find(user_id)
            .first::<BrowserSessionModel>(&mut connection)
            .await
        {
            Ok(row) => {
                let state: BrowserSessionState = serde_json::from_str(&row.state)
                    .context("failed to deserialize browser session state")?;
                Ok(Some(BrowserSessionRecord {
                    user_id: row.user_id,
                    state,
                    updated_at: from_naive(row.updated_at),
                }))
            }
            Err(DieselError::NotFound) => Ok(None),
            Err(err) => Err(anyhow!("failed to load browser session: {err}")),
        }
    }

    async fn delete_session(&self, user_id: &str) -> anyhow::Result<()> {
        let mut connection = self.conn().await?;
        diesel::delete(browser_sessions::table.find(user_id))
            .execute(&mut connection)
            .await
            .context("failed to delete browser session")?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct DieselToolAuthStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselStorePool<Conn>,
}

impl<Conn> DieselToolAuthStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselStorePool<Conn>) -> Self {
        Self { pool }
    }

    async fn conn(&self) -> Result<DieselConn<'_, Conn>> {
        self.pool
            .get()
            .await
            .context("failed to acquire diesel connection")
    }

    async fn load_record(
        &self,
        conn: &mut DieselConn<'_, Conn>,
        user: &str,
        provider_name: &str,
    ) -> Result<Option<IntegrationModel>, AuthError> {
        use crate::schema::integrations::dsl::{integrations as integrations_table, provider};

        integrations_table
            .filter(integrations::user_id.eq(user))
            .filter(provider.eq(provider_name))
            .first::<IntegrationModel>(conn)
            .await
            .optional()
            .map_err(map_store_error)
    }

    async fn ensure_record(
        &self,
        conn: &mut DieselConn<'_, Conn>,
        user: &str,
        provider_name: &str,
    ) -> Result<IntegrationModel, AuthError> {
        if let Some(record) = self.load_record(conn, user, provider_name).await? {
            Ok(record)
        } else {
            let now = current_timestamp();
            let new_record = NewIntegrationModel {
                id: &Uuid::new_v4().to_string(),
                user_id: user,
                provider: provider_name,
                session_data: None,
                secrets_data: None,
                oauth_state: None,
                oauth_state_data: None,
                created_at: now,
                updated_at: now,
            };

            diesel::insert_into(integrations::table)
                .values(&new_record)
                .execute(conn)
                .await
                .map_err(map_store_error)?;

            // Create the record manually instead of fetching to avoid connection issues
            Ok(IntegrationModel {
                id: new_record.id.to_string(),
                user_id: new_record.user_id.to_string(),
                provider: new_record.provider.to_string(),
                session_data: new_record.session_data.map(|s| s.to_string()),
                secrets_data: new_record.secrets_data.map(|s| s.to_string()),
                oauth_state: new_record.oauth_state.map(|s| s.to_string()),
                oauth_state_data: new_record.oauth_state_data.map(|s| s.to_string()),
                created_at: new_record.created_at,
                updated_at: new_record.updated_at,
            })
        }
    }

    async fn save_record(
        &self,
        conn: &mut DieselConn<'_, Conn>,
        record: &IntegrationModel,
    ) -> Result<(), AuthError> {
        use crate::schema::integrations::dsl::{
            id, oauth_state, oauth_state_data, secrets_data, session_data, updated_at,
        };

        diesel::update(integrations::table.filter(id.eq(&record.id)))
            .set((
                session_data.eq(&record.session_data),
                secrets_data.eq(&record.secrets_data),
                oauth_state.eq(&record.oauth_state),
                oauth_state_data.eq(&record.oauth_state_data),
                updated_at.eq(record.updated_at),
            ))
            .execute(conn)
            .await
            .map_err(map_store_error)?;

        Ok(())
    }

    fn session_from_record(record: &IntegrationModel) -> Result<Option<AuthSession>, AuthError> {
        match &record.session_data {
            Some(value) => deserialize_value(value).map(Some),
            None => Ok(None),
        }
    }

    fn secrets_from_record(
        record: &IntegrationModel,
    ) -> Result<HashMap<String, AuthSecret>, AuthError> {
        parse_map(&record.secrets_data)
    }
}

#[async_trait]
impl<Conn> ToolAuthStore for DieselToolAuthStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    async fn get_session(
        &self,
        auth_entity: &str,
        user_id: &str,
    ) -> Result<Option<AuthSession>, AuthError> {
        let user_id = normalize_user_id(user_id)?;
        let mut conn = self.conn().await.map_err(map_store_error)?;

        if let Some(record) = self.load_record(&mut conn, &user_id, auth_entity).await? {
            Self::session_from_record(&record)
        } else {
            Ok(None)
        }
    }

    async fn store_session(
        &self,
        auth_entity: &str,
        user_id: &str,
        session: AuthSession,
    ) -> Result<(), AuthError> {
        let user_id = normalize_user_id(user_id)?;
        let mut conn = self.conn().await.map_err(map_store_error)?;

        let mut record = self.ensure_record(&mut conn, &user_id, auth_entity).await?;
        record.session_data = Some(serialize_value(&session)?);
        record.updated_at = current_timestamp();

        self.save_record(&mut conn, &record).await
    }

    async fn remove_session(&self, auth_entity: &str, user_id: &str) -> Result<bool, AuthError> {
        let user_id = normalize_user_id(user_id)?;
        let mut conn = self.conn().await.map_err(map_store_error)?;

        if let Some(mut record) = self.load_record(&mut conn, &user_id, auth_entity).await? {
            let existed = record.session_data.is_some();
            if existed {
                record.session_data = None;
                record.updated_at = current_timestamp();
                self.save_record(&mut conn, &record).await?;
            }
            Ok(existed)
        } else {
            Ok(false)
        }
    }

    async fn store_secret(
        &self,
        user_id: &str,
        auth_entity: Option<&str>,
        secret: AuthSecret,
    ) -> Result<(), AuthError> {
        let user_id = normalize_user_id(user_id)?;
        let provider = normalize_optional_provider(auth_entity)?;

        let mut conn = self.conn().await.map_err(map_store_error)?;
        let mut record = self.ensure_record(&mut conn, &user_id, &provider).await?;

        let mut secrets = Self::secrets_from_record(&record)?;
        secrets.insert(secret.key.clone(), secret);
        record.secrets_data = store_map(secrets)?;
        record.updated_at = current_timestamp();

        self.save_record(&mut conn, &record).await
    }

    async fn get_secret(
        &self,
        user_id: &str,
        auth_entity: Option<&str>,
        key: &str,
    ) -> Result<Option<AuthSecret>, AuthError> {
        let user_id = normalize_user_id(user_id)?;
        let provider = normalize_optional_provider(auth_entity)?;

        let mut conn = self.conn().await.map_err(map_store_error)?;

        if let Some(record) = self.load_record(&mut conn, &user_id, &provider).await? {
            let secrets = Self::secrets_from_record(&record)?;
            Ok(secrets.get(key).cloned())
        } else {
            Ok(None)
        }
    }

    async fn remove_secret(
        &self,
        user_id: &str,
        auth_entity: Option<&str>,
        key: &str,
    ) -> Result<bool, AuthError> {
        let user_id = normalize_user_id(user_id)?;
        let provider = normalize_optional_provider(auth_entity)?;

        let mut conn = self.conn().await.map_err(map_store_error)?;

        if let Some(mut record) = self.load_record(&mut conn, &user_id, &provider).await? {
            let mut secrets = Self::secrets_from_record(&record)?;
            let existed = secrets.remove(key).is_some();
            record.secrets_data = store_map(secrets)?;
            record.updated_at = current_timestamp();
            self.save_record(&mut conn, &record).await?;
            Ok(existed)
        } else {
            Ok(false)
        }
    }

    async fn store_oauth2_state(&self, state: OAuth2State) -> Result<(), AuthError> {
        let user_id = normalize_user_id(&state.user_id)?;

        let mut conn = self.conn().await.map_err(map_store_error)?;
        let mut record = self
            .ensure_record(&mut conn, &user_id, &state.provider_name)
            .await?;

        let stored = StoredOAuthState {
            redirect_uri: state.redirect_uri.clone(),
            user_id,
            scopes: state.scopes.clone(),
            metadata: state.metadata.clone(),
            created_at: state.created_at.timestamp(),
        };

        record.oauth_state = Some(state.state.clone());
        record.oauth_state_data = Some(serialize_value(&stored)?);
        record.updated_at = current_timestamp();

        self.save_record(&mut conn, &record).await
    }

    async fn get_oauth2_state(&self, state_token: &str) -> Result<Option<OAuth2State>, AuthError> {
        let mut conn = self.conn().await.map_err(map_store_error)?;

        use crate::schema::integrations::dsl::{integrations as integrations_table, oauth_state};

        let record = integrations_table
            .filter(oauth_state.eq(state_token))
            .first::<IntegrationModel>(&mut conn)
            .await
            .optional()
            .map_err(map_store_error)?;

        if let Some(record) = record {
            if let Some(data) = record.oauth_state_data {
                let stored: StoredOAuthState = deserialize_value(&data)?;
                let created_at = Utc
                    .timestamp_opt(stored.created_at, 0)
                    .single()
                    .ok_or_else(|| {
                        AuthError::StoreError("Invalid timestamp in stored state".to_string())
                    })?;

                Ok(Some(OAuth2State {
                    state: state_token.to_string(),
                    provider_name: record.provider,
                    redirect_uri: stored.redirect_uri.clone(),
                    user_id: stored.user_id,
                    scopes: stored.scopes,
                    metadata: stored.metadata,
                    created_at,
                }))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    async fn remove_oauth2_state(&self, state_token: &str) -> Result<(), AuthError> {
        let mut conn = self.conn().await.map_err(map_store_error)?;

        use crate::schema::integrations::dsl::{integrations as integrations_table, oauth_state};

        if let Some(mut record) = integrations_table
            .filter(oauth_state.eq(state_token))
            .first::<IntegrationModel>(&mut conn)
            .await
            .optional()
            .map_err(map_store_error)?
        {
            record.oauth_state = None;
            record.oauth_state_data = None;
            record.updated_at = current_timestamp();
            self.save_record(&mut conn, &record).await?;
        }

        Ok(())
    }

    async fn list_secrets(&self, user_id: &str) -> Result<HashMap<String, AuthSecret>, AuthError> {
        let user_id = normalize_user_id(user_id)?;
        let mut conn = self.conn().await.map_err(map_store_error)?;

        use crate::schema::integrations::dsl::{
            integrations as integrations_table, user_id as col_user_id,
        };

        let records = integrations_table
            .filter(col_user_id.eq(&user_id))
            .load::<IntegrationModel>(&mut conn)
            .await
            .map_err(map_store_error)?;

        let mut secrets = HashMap::new();
        for record in records {
            let provider = record.provider.clone();
            let entries = Self::secrets_from_record(&record)?;
            for (key, secret) in entries {
                secrets.insert(format!("{}::{}", provider, key), secret);
            }
        }

        Ok(secrets)
    }

    async fn list_sessions(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, AuthSession>, AuthError> {
        let user_id = normalize_user_id(user_id)?;
        let mut conn = self.conn().await.map_err(map_store_error)?;

        use crate::schema::integrations::dsl::{
            integrations as integrations_table, user_id as col_user_id,
        };

        let records = integrations_table
            .filter(col_user_id.eq(&user_id))
            .load::<IntegrationModel>(&mut conn)
            .await
            .map_err(map_store_error)?;

        let mut sessions = HashMap::new();
        for record in records {
            if let Some(session) = Self::session_from_record(&record)? {
                sessions.insert(record.provider.clone(), session);
            }
        }

        Ok(sessions)
    }
}

#[derive(Clone)]
pub struct DieselExternalToolCallsStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselStorePool<Conn>,
    pending_channels: Arc<DashMap<String, oneshot::Sender<ToolResponse>>>,
}

impl<Conn> DieselExternalToolCallsStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselStorePool<Conn>) -> Self {
        Self {
            pool,
            pending_channels: Arc::new(DashMap::new()),
        }
    }

    async fn conn(&self) -> Result<DieselConn<'_, Conn>> {
        self.pool
            .get()
            .await
            .context("failed to acquire diesel connection")
    }
}

impl<Conn> std::fmt::Debug for DieselExternalToolCallsStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DieselExternalToolCallsStore")
            .field("pending", &self.pending_channels.len())
            .finish()
    }
}

#[async_trait]
impl<Conn> ExternalToolCallsStore for DieselExternalToolCallsStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    async fn register_external_tool_call(
        &self,
        session_id: &str,
    ) -> Result<oneshot::Receiver<ToolResponse>> {
        let mut connection = self.conn().await?;
        let new_call = NewExternalToolCallModel {
            id: session_id,
            status: "pending",
            request: None,
            response: None,
            created_at: now_naive(),
            updated_at: now_naive(),
            locked_at: None,
        };

        if let Err(err) = diesel::insert_into(external_tool_calls::table)
            .values(&new_call)
            .execute(&mut connection)
            .await
        {
            if matches!(
                err,
                DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, _)
            ) {
                return Err(anyhow!(
                    "external tool call {session_id} is already registered"
                ));
            }
            return Err(err).context("failed to register external tool call");
        }

        let (sender, receiver) = oneshot::channel();
        if self
            .pending_channels
            .insert(session_id.to_string(), sender)
            .is_some()
        {
            warn!(
                "duplicate registration detected for external tool call {session_id}, replacing previous receiver"
            );
        }
        Ok(receiver)
    }

    async fn complete_external_tool_call(
        &self,
        session_id: &str,
        tool_response: ToolResponse,
    ) -> Result<()> {
        let mut connection = self.conn().await?;
        let response_json =
            serde_json::to_string(&tool_response).context("failed to serialize tool response")?;

        let changes = ExternalToolCallChangeset {
            status: Some("completed"),
            request: None,
            response: Some(Some(&response_json)),
            updated_at: now_naive(),
            locked_at: Some(None),
        };

        diesel::update(external_tool_calls::table.find(session_id))
            .set(&changes)
            .execute(&mut connection)
            .await
            .context("failed to update external tool call")?;

        if let Some((_, sender)) = self.pending_channels.remove(session_id) {
            if sender.send(tool_response).is_err() {
                warn!(
                    "receiver dropped before external tool call {session_id} completed response was delivered"
                );
            }
        } else {
            warn!("no pending receiver when completing external tool call {session_id}");
        }

        Ok(())
    }

    async fn remove_tool_call(&self, session_id: &str) -> Result<()> {
        let mut connection = self.conn().await?;
        diesel::delete(external_tool_calls::table.find(session_id))
            .execute(&mut connection)
            .await
            .context("failed to delete external tool call")?;
        self.pending_channels.remove(session_id);
        Ok(())
    }

    async fn list_pending_tool_calls(&self) -> Result<Vec<String>> {
        let mut connection = self.conn().await?;
        let rows = external_tool_calls::table
            .filter(external_tool_calls::status.eq("pending"))
            .select(external_tool_calls::id)
            .load::<String>(&mut connection)
            .await
            .context("failed to list pending external tool calls")?;
        Ok(rows)
    }
}

#[derive(Clone)]
pub struct DieselPluginCatalogStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselStorePool<Conn>,
}

impl<Conn> DieselPluginCatalogStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselStorePool<Conn>) -> Self {
        Self { pool }
    }

    async fn conn(&self) -> Result<DieselConn<'_, Conn>> {
        self.pool
            .get()
            .await
            .context("failed to acquire diesel connection")
    }

    fn model_to_record(model: PluginCatalogModel) -> Result<PluginMetadataRecord> {
        let artifact: PluginArtifact = serde_json::from_str(&model.artifact_json)
            .context("failed to deserialize plugin artifact")?;
        let updated_at = Utc.from_utc_datetime(&model.updated_at);

        Ok(PluginMetadataRecord {
            package_name: model.package_name,
            version: model.version,
            object_prefix: model.object_prefix,
            entrypoint: model.entrypoint,
            artifact,
            updated_at,
        })
    }
}

#[async_trait]
impl<Conn> PluginCatalogStore for DieselPluginCatalogStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    async fn list_plugins(&self) -> Result<Vec<PluginMetadataRecord>> {
        let mut connection = self.conn().await?;
        let rows = plugin_catalog::table
            .order(plugin_catalog::package_name.asc())
            .load::<PluginCatalogModel>(&mut connection)
            .await
            .context("failed to list plugin metadata")?;

        rows.into_iter().map(Self::model_to_record).collect()
    }

    async fn get_plugin(&self, package_name: &str) -> Result<Option<PluginMetadataRecord>> {
        let mut connection = self.conn().await?;
        let row = plugin_catalog::table
            .find(package_name)
            .first::<PluginCatalogModel>(&mut connection)
            .await
            .optional()
            .context("failed to load plugin metadata")?;

        row.map(Self::model_to_record).transpose()
    }

    async fn upsert_plugin(&self, record: &PluginMetadataRecord) -> Result<()> {
        let mut connection = self.conn().await?;
        let timestamp = now_naive();

        let mut artifact = record.artifact.clone();
        if artifact.path.as_os_str().is_empty() {
            artifact.path = std::path::PathBuf::from(&record.object_prefix);
        }
        let artifact_json =
            serde_json::to_string(&artifact).context("failed to serialize plugin artifact")?;

        let insert = NewPluginCatalogModel {
            package_name: &record.package_name,
            version: record.version.as_deref(),
            object_prefix: &record.object_prefix,

            entrypoint: record.entrypoint.as_deref(),
            artifact_json: &artifact_json,
            updated_at: timestamp,
        };

        let changes = PluginCatalogChangeset {
            version: record.version.as_deref(),
            object_prefix: &record.object_prefix,

            entrypoint: record.entrypoint.as_deref(),
            artifact_json: &artifact_json,
            updated_at: timestamp,
        };

        diesel::insert_into(plugin_catalog::table)
            .values(&insert)
            .on_conflict(plugin_catalog::package_name)
            .do_update()
            .set(&changes)
            .execute(&mut connection)
            .await
            .context("failed to upsert plugin metadata")?;

        Ok(())
    }

    async fn remove_plugin(&self, package_name: &str) -> Result<()> {
        let mut connection = self.conn().await?;
        diesel::delete(plugin_catalog::table.find(package_name))
            .execute(&mut connection)
            .await
            .context("failed to delete plugin metadata")?;
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        let mut connection = self.conn().await?;
        diesel::delete(plugin_catalog::table)
            .execute(&mut connection)
            .await
            .context("failed to clear plugin metadata")?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct DieselStoreBuilder<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselStorePool<Conn>,
}

impl<Conn> DieselStoreBuilder<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselStorePool<Conn>) -> Self {
        Self { pool }
    }

    pub fn agent_store(&self) -> DieselAgentStore<Conn> {
        DieselAgentStore::new(self.pool.clone_store_pool())
    }

    pub fn thread_store(&self) -> DieselThreadStore<Conn> {
        DieselThreadStore::new(self.pool.clone_store_pool())
    }

    pub fn task_store(&self) -> DieselTaskStore<Conn> {
        DieselTaskStore::new(self.pool.clone_store_pool())
    }

    pub fn session_store(&self) -> DieselSessionStore<Conn> {
        DieselSessionStore::new(self.pool.clone_store_pool())
    }

    pub fn memory_store(&self) -> DieselMemoryStore<Conn> {
        DieselMemoryStore::new(self.pool.clone_store_pool())
    }

    pub fn scratchpad_store(&self) -> DieselScratchpadStore<Conn> {
        DieselScratchpadStore::new(self.pool.clone_store_pool())
    }
    pub fn tool_auth_store(&self) -> DieselToolAuthStore<Conn> {
        DieselToolAuthStore::new(self.pool.clone_store_pool())
    }

    pub fn external_tool_calls_store(&self) -> DieselExternalToolCallsStore<Conn> {
        DieselExternalToolCallsStore::new(self.pool.clone_store_pool())
    }

    pub fn plugin_catalog_store(&self) -> DieselPluginCatalogStore<Conn> {
        DieselPluginCatalogStore::new(self.pool.clone_store_pool())
    }

    pub fn browser_session_store(&self) -> DieselBrowserSessionStore<Conn> {
        DieselBrowserSessionStore::new(self.pool.clone_store_pool())
    }

    pub fn pool(&self) -> DieselStorePool<Conn> {
        self.pool.clone()
    }

    pub fn prompt_template_store(&self) -> DieselPromptTemplateStore<Conn> {
        DieselPromptTemplateStore::new(self.pool.clone_store_pool())
    }

    pub fn secret_store(&self) -> DieselSecretStore<Conn> {
        DieselSecretStore::new(self.pool.clone_store_pool())
    }
}

// ========== Prompt Template Store ==========

// ========== Secret Store ==========

fn to_secret_record(model: SecretModel) -> SecretRecord {
    SecretRecord {
        id: model.id,
        key: model.key,
        value: model.value,
        created_at: from_naive(model.created_at),
        updated_at: from_naive(model.updated_at),
    }
}

pub struct DieselSecretStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselStorePool<Conn>,
}

impl<Conn> DieselSecretStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselStorePool<Conn>) -> Self {
        Self { pool }
    }

    async fn conn(&self) -> Result<DieselConn<'_, Conn>> {
        self.pool
            .get()
            .await
            .context("failed to acquire diesel connection for secrets")
    }
}

#[async_trait]
impl<Conn> SecretStore for DieselSecretStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    async fn list(&self) -> Result<Vec<SecretRecord>> {
        use crate::schema::secrets::dsl::*;
        let mut conn = self.conn().await?;
        let results = secrets
            .select(SecretModel::as_select())
            .load::<SecretModel>(&mut conn)
            .await?;
        Ok(results.into_iter().map(to_secret_record).collect())
    }

    async fn get(&self, secret_key: &str) -> Result<Option<SecretRecord>> {
        use crate::schema::secrets::dsl::*;
        let mut conn = self.conn().await?;
        let result = secrets
            .filter(key.eq(secret_key))
            .select(SecretModel::as_select())
            .first::<SecretModel>(&mut conn)
            .await
            .optional()?;
        Ok(result.map(to_secret_record))
    }

    async fn create(&self, secret: NewSecret) -> Result<SecretRecord> {
        use crate::schema::secrets::dsl::*;
        let mut conn = self.conn().await?;
        let now = Utc::now().naive_utc();
        let new_id = Uuid::new_v4().to_string();

        let model = NewSecretModel {
            id: &new_id,
            key: &secret.key,
            value: &secret.value,
            created_at: now,
            updated_at: now,
        };

        diesel::insert_into(secrets)
            .values(&model)
            .execute(&mut conn)
            .await?;

        let result = secrets
            .filter(id.eq(&new_id))
            .select(SecretModel::as_select())
            .first::<SecretModel>(&mut conn)
            .await?;

        Ok(to_secret_record(result))
    }

    async fn update(&self, secret_key: &str, new_value: &str) -> Result<SecretRecord> {
        use crate::schema::secrets::dsl::*;
        let mut conn = self.conn().await?;
        let now = Utc::now().naive_utc();

        diesel::update(secrets.filter(key.eq(secret_key)))
            .set((value.eq(new_value), updated_at.eq(now)))
            .execute(&mut conn)
            .await?;

        let result = secrets
            .filter(key.eq(secret_key))
            .select(SecretModel::as_select())
            .first::<SecretModel>(&mut conn)
            .await?;

        Ok(to_secret_record(result))
    }

    async fn delete(&self, secret_key: &str) -> Result<()> {
        use crate::schema::secrets::dsl::*;
        let mut conn = self.conn().await?;
        diesel::delete(secrets.filter(key.eq(secret_key)))
            .execute(&mut conn)
            .await?;
        Ok(())
    }
}

fn to_prompt_template_record(model: PromptTemplateModel) -> PromptTemplateRecord {
    PromptTemplateRecord {
        id: model.id,
        name: model.name,
        template: model.template,
        description: model.description,
        version: model.version,
        is_system: model.is_system != 0,
        created_at: from_naive(model.created_at),
        updated_at: from_naive(model.updated_at),
    }
}

#[derive(Clone)]
pub struct DieselPromptTemplateStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pool: DieselStorePool<Conn>,
}

impl<Conn> DieselPromptTemplateStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    pub fn new(pool: DieselStorePool<Conn>) -> Self {
        Self { pool }
    }

    async fn conn(&self) -> Result<DieselConn<'_, Conn>> {
        self.pool
            .get()
            .await
            .context("failed to acquire diesel connection for prompt templates")
    }
}

#[async_trait]
impl<Conn> PromptTemplateStore for DieselPromptTemplateStore<Conn>
where
    Conn: DieselBackendConnection,
    diesel::dsl::select<diesel::dsl::AsExprOf<i32, diesel::sql_types::Integer>>: ExecuteDsl<Conn>,
    diesel::query_builder::SqlQuery: QueryFragment<<Conn as AsyncConnectionCore>::Backend>,
    <Conn as AsyncConnectionCore>::Backend: diesel::backend::DieselReserveSpecialization,
{
    async fn list(&self) -> Result<Vec<PromptTemplateRecord>> {
        use crate::schema::prompt_templates::dsl::*;
        let mut conn = self.conn().await?;
        let results = prompt_templates
            .select(PromptTemplateModel::as_select())
            .load::<PromptTemplateModel>(&mut conn)
            .await?;
        Ok(results.into_iter().map(to_prompt_template_record).collect())
    }

    async fn get(&self, template_id: &str) -> Result<Option<PromptTemplateRecord>> {
        use crate::schema::prompt_templates::dsl::*;
        let mut conn = self.conn().await?;
        let result = prompt_templates
            .filter(id.eq(template_id))
            .select(PromptTemplateModel::as_select())
            .first::<PromptTemplateModel>(&mut conn)
            .await
            .optional()?;
        Ok(result.map(to_prompt_template_record))
    }

    async fn create(&self, template_data: NewPromptTemplate) -> Result<PromptTemplateRecord> {
        use crate::schema::prompt_templates::dsl::*;
        let mut conn = self.conn().await?;
        let now = Utc::now().naive_utc();
        let new_id = Uuid::new_v4().to_string();

        let model = NewPromptTemplateModel {
            id: &new_id,
            name: &template_data.name,
            template: &template_data.template,
            description: template_data.description.as_deref(),
            version: template_data.version.as_deref(),
            is_system: if template_data.is_system { 1 } else { 0 },
            created_at: now,
            updated_at: now,
        };

        diesel::insert_into(prompt_templates)
            .values(&model)
            .execute(&mut conn)
            .await?;

        let result = prompt_templates
            .filter(id.eq(&new_id))
            .select(PromptTemplateModel::as_select())
            .first::<PromptTemplateModel>(&mut conn)
            .await?;

        Ok(to_prompt_template_record(result))
    }

    async fn update(
        &self,
        template_id: &str,
        update_data: UpdatePromptTemplate,
    ) -> Result<PromptTemplateRecord> {
        use crate::schema::prompt_templates::dsl::*;
        let mut conn = self.conn().await?;
        let now = Utc::now().naive_utc();

        // Check if it's a system template
        let existing = prompt_templates
            .filter(id.eq(template_id))
            .select(PromptTemplateModel::as_select())
            .first::<PromptTemplateModel>(&mut conn)
            .await?;

        if existing.is_system != 0 {
            return Err(anyhow!("system templates cannot be modified"));
        }

        diesel::update(prompt_templates.filter(id.eq(template_id)))
            .set((
                name.eq(&update_data.name),
                template.eq(&update_data.template),
                description.eq(update_data.description.as_deref()),
                updated_at.eq(now),
            ))
            .execute(&mut conn)
            .await?;

        let result = prompt_templates
            .filter(id.eq(template_id))
            .select(PromptTemplateModel::as_select())
            .first::<PromptTemplateModel>(&mut conn)
            .await?;

        Ok(to_prompt_template_record(result))
    }

    async fn delete(&self, template_id: &str) -> Result<()> {
        use crate::schema::prompt_templates::dsl::*;
        let mut conn = self.conn().await?;

        // Check if it's a system template
        let existing = prompt_templates
            .filter(id.eq(template_id))
            .select(PromptTemplateModel::as_select())
            .first::<PromptTemplateModel>(&mut conn)
            .await
            .optional()?;

        if let Some(record) = existing {
            if record.is_system != 0 {
                return Err(anyhow!("system templates cannot be deleted"));
            }

            diesel::delete(prompt_templates.filter(id.eq(template_id)))
                .execute(&mut conn)
                .await?;
        }

        Ok(())
    }

    async fn clone_template(&self, template_id: &str) -> Result<PromptTemplateRecord> {
        use crate::schema::prompt_templates::dsl::*;
        let mut conn = self.conn().await?;

        // Fetch the source template
        let source_tpl = prompt_templates
            .filter(id.eq(template_id))
            .select(PromptTemplateModel::as_select())
            .first::<PromptTemplateModel>(&mut conn)
            .await?;

        let now = Utc::now().naive_utc();
        let new_id = uuid::Uuid::new_v4().to_string();
        let new_name = format!("Clone of {}", source_tpl.name);

        let new_model = NewPromptTemplateModel {
            id: &new_id,
            name: &new_name,
            template: &source_tpl.template,
            description: source_tpl.description.as_deref(),
            version: source_tpl.version.as_deref(),
            is_system: 0,
            created_at: now,
            updated_at: now,
        };

        diesel::insert_into(prompt_templates)
            .values(&new_model)
            .execute(&mut conn)
            .await?;

        let result = prompt_templates
            .filter(id.eq(&new_id))
            .select(PromptTemplateModel::as_select())
            .first::<PromptTemplateModel>(&mut conn)
            .await?;

        Ok(to_prompt_template_record(result))
    }

    async fn sync_system_templates(&self, templates_to_sync: Vec<NewPromptTemplate>) -> Result<()> {
        use crate::schema::prompt_templates::dsl::*;
        let mut conn = self.conn().await?;
        let now = Utc::now().naive_utc();

        for tpl in templates_to_sync {
            // Check if exists by name (system templates are identified by name for syncing)
            let existing = prompt_templates
                .filter(name.eq(&tpl.name))
                .filter(is_system.eq(1))
                .select(PromptTemplateModel::as_select())
                .first::<PromptTemplateModel>(&mut conn)
                .await
                .optional()?;

            if let Some(existing_model) = existing {
                // Update if content changed
                if existing_model.template != tpl.template {
                    diesel::update(prompt_templates.filter(id.eq(existing_model.id)))
                        .set((
                            template.eq(&tpl.template),
                            description.eq(tpl.description.as_deref()),
                            version.eq(tpl.version.as_deref()),
                            updated_at.eq(now),
                        ))
                        .execute(&mut conn)
                        .await?;
                }
            } else {
                // Create new
                let new_id = Uuid::new_v4().to_string();
                let model = NewPromptTemplateModel {
                    id: &new_id,
                    name: &tpl.name,
                    template: &tpl.template,
                    description: tpl.description.as_deref(),
                    version: tpl.version.as_deref(),
                    is_system: 1,
                    created_at: now,
                    updated_at: now,
                };

                diesel::insert_into(prompt_templates)
                    .values(&model)
                    .execute(&mut conn)
                    .await?;
            }
        }

        Ok(())
    }
}

#[cfg(feature = "sqlite")]
impl DieselStoreBuilder<SqliteConnectionWrapper> {
    pub async fn sqlite(database_url: &str, max_connections: u32) -> Result<Self> {
        Ok(Self {
            pool: DieselStorePool::new_sqlite(database_url, max_connections).await?,
        })
    }
}

#[cfg(all(not(feature = "sqlite"), feature = "postgres"))]
impl DieselStoreBuilder<diesel_async::AsyncPgConnection> {
    pub async fn postgres(database_url: &str, max_connections: u32) -> Result<Self> {
        Ok(Self {
            pool: DieselStorePool::new_postgres(database_url, max_connections).await?,
        })
    }
}
