use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents the Agent-to-Agent (A2A) specification version.
pub const A2A_VERSION: &str = "0.10.0";

/// Describes an agent's capabilities, skills, and metadata, serving as a public profile.
/// See: https://google.github.io/A2A/specification/#agentcard-object-structure
#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    /// The version of the A2A specification this agent adheres to.
    pub version: String,
    /// The agent's unique name.
    pub name: String,
    /// A short description of the agent's purpose.
    pub description: String,
    /// The URL where the agent can be reached.
    pub url: String,
    /// A URL to an icon for the agent.
    #[serde(default)]
    pub icon_url: Option<String>,
    /// A URL to the agent's documentation.
    #[serde(default)]
    pub documentation_url: Option<String>,
    /// Information about the agent's provider.
    #[serde(default)]
    pub provider: Option<AgentProvider>,
    /// The preferred transport method for communicating with the agent.
    #[serde(default)]
    pub preferred_transport: Option<String>,
    /// The agent's capabilities.
    pub capabilities: AgentCapabilities,
    /// The default input modes the agent accepts.
    pub default_input_modes: Vec<String>,
    /// The default output modes the agent produces.
    pub default_output_modes: Vec<String>,
    /// The skills the agent possesses.
    pub skills: Vec<AgentSkill>,
    /// The security schemes supported by the agent.
    #[serde(default)]
    pub security_schemes: HashMap<String, SecurityScheme>,
    /// The security requirements for the agent.
    #[serde(default)]
    pub security: Vec<HashMap<String, Vec<String>>>,
}

/// Provides information about the organization or individual that created the agent.
#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentProvider {
    /// The name of the organization.
    pub organization: String,
    /// A URL to the provider's website.
    pub url: String,
}

/// Defines the agent's supported features and extensions.
#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    /// Whether the agent supports streaming responses.
    #[serde(default)]
    pub streaming: bool,
    /// Whether the agent can send push notifications.
    #[serde(default)]
    pub push_notifications: bool,
    /// Whether the agent can provide a history of state transitions.
    #[serde(default)]
    pub state_transition_history: bool,
    /// Any extensions the agent supports.
    #[serde(default)]
    pub extensions: Vec<AgentExtension>,
}

/// Describes a custom extension supported by the agent.
#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentExtension {
    /// A URI that uniquely identifies the extension.
    pub uri: String,
    /// A description of the extension.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether the extension is required for the agent to function.
    #[serde(default)]
    pub required: bool,
    /// Any parameters the extension requires.
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

/// Describes a specific skill or capability of the agent.
#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    /// A unique identifier for the skill.
    pub id: String,
    /// The name of the skill.
    pub name: String,
    /// A description of what the skill does.
    pub description: String,
    /// Tags for categorizing the skill.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Examples of how to use the skill.
    #[serde(default)]
    pub examples: Vec<String>,
    /// The input modes the skill accepts, overriding agent defaults.
    #[serde(default)]
    pub input_modes: Option<Vec<String>>,
    /// The output modes the skill produces, overriding agent defaults.
    #[serde(default)]
    pub output_modes: Option<Vec<String>>,
}

/// Defines a security scheme for authenticating with the agent.
#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SecurityScheme {
    ApiKey(APIKeySecurityScheme),
    Http(HTTPAuthSecurityScheme),
    Oauth2(OAuth2SecurityScheme),
    OpenIdConnect(OpenIdConnectSecurityScheme),
}

/// An API key-based security scheme.
#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct APIKeySecurityScheme {
    /// The name of the header, query, or cookie parameter to be used.
    pub name: String,
    /// The location of the API key.
    #[serde(rename = "in")]
    pub location: String,
    /// A description of the security scheme.
    #[serde(default)]
    pub description: Option<String>,
}

/// An HTTP authentication-based security scheme.
#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HTTPAuthSecurityScheme {
    /// The name of the HTTP Authorization scheme to be used.
    pub scheme: String,
    /// A hint to the client about the format of the bearer token.
    #[serde(default)]
    pub bearer_format: Option<String>,
    /// A description of the security scheme.
    #[serde(default)]
    pub description: Option<String>,
}

/// An OAuth2-based security scheme.
#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OAuth2SecurityScheme {
    /// The OAuth2 flows supported by this scheme.
    pub flows: OAuthFlows,
    /// A description of the security scheme.
    #[serde(default)]
    pub description: Option<String>,
}

/// An OpenID Connect-based security scheme.
#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OpenIdConnectSecurityScheme {
    /// The OpenID Connect discovery URL.
    pub open_id_connect_url: String,
    /// A description of the security scheme.
    #[serde(default)]
    pub description: Option<String>,
}

/// Defines the OAuth2 flows.
#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct OAuthFlows {
    #[serde(default)]
    pub implicit: Option<ImplicitOAuthFlow>,
    #[serde(default)]
    pub password: Option<PasswordOAuthFlow>,
    #[serde(default)]
    pub client_credentials: Option<ClientCredentialsOAuthFlow>,
    #[serde(default)]
    pub authorization_code: Option<AuthorizationCodeOAuthFlow>,
}

/// The implicit OAuth2 flow.
#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ImplicitOAuthFlow {
    pub authorization_url: String,
    #[serde(default)]
    pub refresh_url: Option<String>,
    pub scopes: HashMap<String, String>,
}

/// The password-based OAuth2 flow.
#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PasswordOAuthFlow {
    pub token_url: String,
    #[serde(default)]
    pub refresh_url: Option<String>,
    pub scopes: HashMap<String, String>,
}

/// The client credentials OAuth2 flow.
#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ClientCredentialsOAuthFlow {
    pub token_url: String,
    #[serde(default)]
    pub refresh_url: Option<String>,
    pub scopes: HashMap<String, String>,
}

/// The authorization code OAuth2 flow.
#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationCodeOAuthFlow {
    pub authorization_url: String,
    pub token_url: String,
    #[serde(default)]
    pub refresh_url: Option<String>,
    pub scopes: HashMap<String, String>,
}

// JSON-RPC Types

/// A JSON-RPC request object.
#[derive(Serialize, Deserialize, Debug)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
}

/// A JSON-RPC response object.
#[derive(Serialize, Debug)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: Option<serde_json::Value>,
}

/// A JSON-RPC error object.
#[derive(Serialize, Debug)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// A2A Method Params

/// Parameters for the `message/send` method.
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MessageSendParams {
    pub message: Message,
    #[serde(default)]
    pub configuration: Option<MessageSendConfiguration>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Configuration for sending a message.
#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct MessageSendConfiguration {
    pub accepted_output_modes: Vec<String>,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default)]
    pub history_length: Option<u32>,
    #[serde(default)]
    pub push_notification_config: Option<PushNotificationConfig>,
}

/// A message exchanged between a user and an agent.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub message_id: String,
    pub role: Role,
    pub parts: Vec<Part>,
    #[serde(default)]
    pub context_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub reference_task_ids: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// The role of the message sender.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    User,
    Agent,
}

/// A part of a message.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind")]
pub enum Part {
    #[serde(rename = "text")]
    Text(TextPart),
    #[serde(rename = "file")]
    File(FilePart),
    #[serde(rename = "data")]
    Data(DataPart),
}

/// A text part of a message.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TextPart {
    pub text: String,
}

/// A file part of a message.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FilePart {
    pub file: FileObject,
}

/// A file object, which can be represented by a URI or by its raw bytes.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum FileObject {
    WithUri { uri: String },
    WithBytes { bytes: String },
}

/// A data part of a message, containing arbitrary JSON data.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DataPart {
    pub data: serde_json::Value,
}

/// Configuration for push notifications.
#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct PushNotificationConfig {
    pub url: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

/// Parameters for methods that operate on a task by its ID.
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TaskIdParams {
    pub id: String,
}

/// Represents a task being executed by the agent.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub kind: String,
    #[serde(rename = "contextId")]
    pub context_id: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub history: Vec<Message>,
}

/// The status of a task.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub state: TaskState,
    #[serde(default)]
    pub message: Option<Message>,
    #[serde(default)]
    pub timestamp: Option<String>,
}

/// The state of a task.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub enum TaskState {
    Submitted,
    Working,
    InputRequired,
    Completed,
    Canceled,
    Failed,
    Rejected,
    AuthRequired,
    Unknown,
}

/// An artifact produced by a task.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub artifact_id: String,
    pub parts: Vec<Part>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}
