use crate::a2a::{AgentCapabilities, AgentProvider, SecurityScheme};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct StoreConfig {
    /// Metadata store (agent_config, tool_auth) - persistent across sessions
    #[serde(default)]
    pub metadata: MetadataStoreConfig,

    /// Memory store configuration (for vector search) - persistent cross-session memory
    #[serde(default)]
    pub memory: Option<MemoryStoreConfig>,

    /// Session stores (threads, tasks, scratchpad, session) - ephemeral by default
    #[serde(default)]
    pub session: SessionStoreConfig,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq, Hash)]
#[serde(tag = "type", content = "config", rename_all = "lowercase")]
pub enum StoreType {
    #[default]
    Sqlite,
    Postgres,
    Custom {
        name: String,
    },
}

impl StoreType {
    pub fn label(&self) -> &str {
        match self {
            StoreType::Sqlite => "sqlite",
            StoreType::Postgres => "postgres",
            StoreType::Custom { name } => name.as_str(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MetadataStoreConfig {
    #[serde(default)]
    pub store_type: StoreType,
    #[serde(default)]
    pub db_config: Option<DbConnectionConfig>,
}

impl Default for MetadataStoreConfig {
    fn default() -> Self {
        Self {
            store_type: StoreType::Sqlite,
            db_config: Some(DbConnectionConfig::default()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryStoreConfig {
    #[serde(default)]
    pub store_type: StoreType,
    #[serde(default)]
    pub db_config: Option<DbConnectionConfig>,
    #[serde(default = "default_embedding_dimension")]
    pub embedding_dimension: usize,
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: f32,
    #[serde(default = "default_max_results")]
    pub max_results: usize,
    pub openai_api_key: Option<String>,
}

impl Default for MemoryStoreConfig {
    fn default() -> Self {
        Self {
            store_type: StoreType::Sqlite,
            db_config: Some(DbConnectionConfig::default()),
            embedding_dimension: default_embedding_dimension(),
            similarity_threshold: default_similarity_threshold(),
            max_results: default_max_results(),
            openai_api_key: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStoreConfig {
    /// If true, creates a new ephemeral in-memory SQLite connection for each thread execution
    /// When ephemeral, store_type and db_config are ignored (always uses in-memory SQLite)
    #[serde(default = "default_ephemeral")]
    pub ephemeral: bool,
    /// Store type (only used when ephemeral=false)
    #[serde(default)]
    pub store_type: StoreType,
    /// Database config (only used when ephemeral=false)
    #[serde(default)]
    pub db_config: Option<DbConnectionConfig>,
}

fn default_ephemeral() -> bool {
    true
}

impl Default for SessionStoreConfig {
    fn default() -> Self {
        Self {
            ephemeral: true,
            store_type: StoreType::Sqlite,
            db_config: Some(DbConnectionConfig::default()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PostgresConfig {
    pub database_url: String,
    #[serde(default = "default_postgres_max_connections")]
    pub max_connections: u32,
    #[serde(default = "default_postgres_min_connections")]
    pub min_connections: u32,
    #[serde(default = "default_postgres_connection_timeout")]
    pub connection_timeout: u64,
    #[serde(default = "default_postgres_idle_timeout")]
    pub idle_timeout: u64,
}

fn default_postgres_max_connections() -> u32 {
    3
}

fn default_postgres_min_connections() -> u32 {
    1
}

fn default_postgres_connection_timeout() -> u64 {
    30
}

fn default_postgres_idle_timeout() -> u64 {
    600
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbConnectionConfig {
    #[serde(default = "default_database_url")]
    pub database_url: String,
    #[serde(default = "default_connections")]
    pub max_connections: u32,
}

impl Default for DbConnectionConfig {
    fn default() -> Self {
        DbConnectionConfig {
            database_url: default_database_url(),
            max_connections: default_connections(),
        }
    }
}

fn default_database_url() -> String {
    // "file::memory:".to_string()
    ".distri/distri.db".to_string()
}

fn default_connections() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "config", rename_all = "lowercase")]
pub enum ObjectStorageConfig {
    /// Local filesystem storage
    #[serde(rename = "filesystem")]
    FileSystem {
        /// Base directory for storing objects
        base_path: String,
    },

    /// Google Cloud Storage
    #[serde(rename = "gcs")]
    GoogleCloudStorage {
        bucket: String,
        project_id: String,
        /// Path to service account key file
        service_account_key: Option<String>,
        /// Base64 encoded service account key
        service_account_key_base64: Option<String>,
    },

    /// AWS S3 (future implementation)
    #[serde(rename = "s3")]
    S3 {
        bucket: String,
        region: String,
        endpoint: Option<String>,
        access_key_id: String,
        secret_access_key: String,
        path_style: Option<bool>,
    },
}

impl Default for ObjectStorageConfig {
    fn default() -> Self {
        Self::FileSystem {
            base_path: "./data/artifacts".to_string(),
        }
    }
}

fn default_embedding_dimension() -> usize {
    1536
}

fn default_similarity_threshold() -> f32 {
    0.7
}

fn default_max_results() -> usize {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServerConfig {
    #[serde(default = "default_server_url")]
    pub base_url: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default = "default_agent_provider")]
    pub agent_provider: AgentProvider,
    #[serde(default)]
    pub default_input_modes: Vec<String>,
    #[serde(default)]
    pub default_output_modes: Vec<String>,
    #[serde(default)]
    pub security_schemes: HashMap<String, SecurityScheme>,
    #[serde(default)]
    pub security: Vec<HashMap<String, Vec<String>>>,
    #[serde(default = "default_capabilities")]
    pub capabilities: AgentCapabilities,
    #[serde(default = "default_preferred_transport")]
    pub preferred_transport: Option<String>,
    #[serde(default = "default_documentation_url")]
    pub documentation_url: Option<String>,
}

fn default_capabilities() -> AgentCapabilities {
    AgentCapabilities {
        streaming: true,
        push_notifications: false,
        state_transition_history: true,
        extensions: vec![],
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            base_url: default_server_url(),
            agent_provider: default_agent_provider(),
            default_input_modes: vec![],
            default_output_modes: vec![],
            security_schemes: HashMap::new(),
            security: vec![],
            capabilities: default_capabilities(),
            port: None,
            host: None,
            preferred_transport: default_preferred_transport(),
            documentation_url: default_documentation_url(),
        }
    }
}

fn default_agent_provider() -> AgentProvider {
    AgentProvider {
        organization: "Distri".to_string(),
        url: "https://distri.ai".to_string(),
    }
}

fn default_server_url() -> String {
    "http://localhost:8081/v1".to_string()
}

fn default_documentation_url() -> Option<String> {
    Some("https://github.com/distrihub/distri".to_string())
}

fn default_preferred_transport() -> Option<String> {
    Some("JSONRPC".to_string())
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct ExternalMcpServer {
    pub name: String,
    #[serde(default, flatten)]
    pub config: crate::McpServerMetadata,
}
