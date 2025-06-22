use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub version: String,
    pub name: String,
    pub description: String,
    pub url: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub documentation_url: Option<String>,
    #[serde(default)]
    pub provider: Option<AgentProvider>,
    #[serde(default)]
    pub preferred_transport: Option<String>,
    pub capabilities: AgentCapabilities,
    pub default_input_modes: Vec<String>,
    pub default_output_modes: Vec<String>,
    pub skills: Vec<AgentSkill>,
    #[serde(default)]
    pub security_schemes: HashMap<String, SecurityScheme>,
    #[serde(default)]
    pub security: Vec<HashMap<String, Vec<String>>>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentProvider {
    pub organization: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub push_notifications: bool,
    #[serde(default)]
    pub state_transition_history: bool,
    #[serde(default)]
    pub extensions: Vec<AgentExtension>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentExtension {
    pub uri: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub input_modes: Option<Vec<String>>,
    #[serde(default)]
    pub output_modes: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SecurityScheme {
    ApiKey(APIKeySecurityScheme),
    Http(HTTPAuthSecurityScheme),
    Oauth2(OAuth2SecurityScheme),
    OpenIdConnect(OpenIdConnectSecurityScheme),
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct APIKeySecurityScheme {
    pub name: String,
    #[serde(rename = "in")]
    pub location: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HTTPAuthSecurityScheme {
    pub scheme: String,
    #[serde(default)]
    pub bearer_format: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OAuth2SecurityScheme {
    pub flows: OAuthFlows,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OpenIdConnectSecurityScheme {
    pub open_id_connect_url: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
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

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImplicitOAuthFlow {
    pub authorization_url: String,
    #[serde(default)]
    pub refresh_url: Option<String>,
    pub scopes: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasswordOAuthFlow {
    pub token_url: String,
    #[serde(default)]
    pub refresh_url: Option<String>,
    pub scopes: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClientCredentialsOAuthFlow {
    pub token_url: String,
    #[serde(default)]
    pub refresh_url: Option<String>,
    pub scopes: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationCodeOAuthFlow {
    pub authorization_url: String,
    pub token_url: String,
    #[serde(default)]
    pub refresh_url: Option<String>,
    pub scopes: HashMap<String, String>,
}

// JSON-RPC Types
#[derive(Serialize, Deserialize, Debug)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
}

#[derive(Serialize, Debug)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: Option<serde_json::Value>,
}

#[derive(Serialize, Debug)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// A2A Method Params
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MessageSendParams {
    pub message: Message,
    #[serde(default)]
    pub configuration: Option<MessageSendConfiguration>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

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

#[derive(Serialize, Deserialize, Debug)]
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

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    User,
    Agent,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "kind")]
pub enum Part {
    #[serde(rename = "text")]
    Text(TextPart),
    #[serde(rename = "file")]
    File(FilePart),
    #[serde(rename = "data")]
    Data(DataPart),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TextPart {
    pub text: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FilePart {
    pub file: FileObject,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum FileObject {
    WithUri { uri: String },
    WithBytes { bytes: String },
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DataPart {
    pub data: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct PushNotificationConfig {
    pub url: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TaskIdParams {
    pub id: String,
}

#[derive(Serialize, Deserialize, Debug)]
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

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub state: TaskState,
    #[serde(default)]
    pub message: Option<Message>,
    #[serde(default)]
    pub timestamp: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
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

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub artifact_id: String,
    pub parts: Vec<Part>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}
