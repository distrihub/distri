//! A2A 1.0 core types: messages, parts, tasks, artifacts, stream events and
//! request/response params. Mirrors `a2aproject/a2a-rs` (`a2a/src/types.rs`,
//! `a2a/src/event.rs`) with the same JSON (ProtoJSON) wire names.
//!
//! Decoding is lenient for one release: the 0.2/0.3 shapes (`kind`-tagged
//! parts and events, lower-case roles and states, millisecond timestamps)
//! are still accepted on input. Encoding is 1.0 only; [`crate::legacy`]
//! turns a 1.0 value back into the 0.x shape for clients that still call the
//! old method names.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// Free-form metadata map (`google.protobuf.Struct`).
pub type Metadata = serde_json::Map<String, Value>;

// ---------------------------------------------------------------------------
// Role
// ---------------------------------------------------------------------------

/// The sender of a message.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Role {
    Unspecified,
    User,
    #[default]
    Agent,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Unspecified => "ROLE_UNSPECIFIED",
            Role::User => "ROLE_USER",
            Role::Agent => "ROLE_AGENT",
        }
    }
}

impl Serialize for Role {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Role {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        match &v {
            Value::String(s) => match s.as_str() {
                "ROLE_USER" | "user" => Ok(Role::User),
                "ROLE_AGENT" | "agent" => Ok(Role::Agent),
                "ROLE_UNSPECIFIED" | "" => Ok(Role::Unspecified),
                other => Err(serde::de::Error::unknown_variant(
                    other,
                    &["ROLE_USER", "ROLE_AGENT", "ROLE_UNSPECIFIED"],
                )),
            },
            Value::Number(n) => match n.as_i64() {
                Some(1) => Ok(Role::User),
                Some(2) => Ok(Role::Agent),
                _ => Ok(Role::Unspecified),
            },
            _ => Err(serde::de::Error::custom("role must be a string")),
        }
    }
}

// ---------------------------------------------------------------------------
// TaskState
// ---------------------------------------------------------------------------

/// The lifecycle state of a task.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum TaskState {
    Unspecified,
    #[default]
    Submitted,
    Working,
    Completed,
    Failed,
    Canceled,
    InputRequired,
    Rejected,
    AuthRequired,
}

impl TaskState {
    /// Terminal states: no further changes.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskState::Completed | TaskState::Failed | TaskState::Canceled | TaskState::Rejected
        )
    }

    /// Interrupted states: the task waits on the client.
    pub fn is_interrupted(&self) -> bool {
        matches!(self, TaskState::InputRequired | TaskState::AuthRequired)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TaskState::Unspecified => "TASK_STATE_UNSPECIFIED",
            TaskState::Submitted => "TASK_STATE_SUBMITTED",
            TaskState::Working => "TASK_STATE_WORKING",
            TaskState::Completed => "TASK_STATE_COMPLETED",
            TaskState::Failed => "TASK_STATE_FAILED",
            TaskState::Canceled => "TASK_STATE_CANCELED",
            TaskState::InputRequired => "TASK_STATE_INPUT_REQUIRED",
            TaskState::Rejected => "TASK_STATE_REJECTED",
            TaskState::AuthRequired => "TASK_STATE_AUTH_REQUIRED",
        }
    }

    /// The spelling distri's 0.x wire used (camelCase: `inputRequired`),
    /// for 0.x callers.
    pub fn legacy_str(&self) -> &'static str {
        match self {
            TaskState::Unspecified => "unknown",
            TaskState::Submitted => "submitted",
            TaskState::Working => "working",
            TaskState::Completed => "completed",
            TaskState::Failed => "failed",
            TaskState::Canceled => "canceled",
            TaskState::InputRequired => "inputRequired",
            TaskState::Rejected => "rejected",
            TaskState::AuthRequired => "authRequired",
        }
    }

    /// Parse either spelling (1.0 `TASK_STATE_*`, 0.x kebab/camel case).
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "TASK_STATE_SUBMITTED" | "submitted" => TaskState::Submitted,
            "TASK_STATE_WORKING" | "working" => TaskState::Working,
            "TASK_STATE_COMPLETED" | "completed" => TaskState::Completed,
            "TASK_STATE_FAILED" | "failed" => TaskState::Failed,
            "TASK_STATE_CANCELED" | "canceled" | "cancelled" => TaskState::Canceled,
            "TASK_STATE_INPUT_REQUIRED" | "input-required" | "inputRequired" => {
                TaskState::InputRequired
            }
            "TASK_STATE_REJECTED" | "rejected" => TaskState::Rejected,
            "TASK_STATE_AUTH_REQUIRED" | "auth-required" | "authRequired" => {
                TaskState::AuthRequired
            }
            "TASK_STATE_UNSPECIFIED" | "" | "unknown" => TaskState::Unspecified,
            _ => return None,
        })
    }
}

impl Serialize for TaskState {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TaskState {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        match &v {
            Value::String(s) => TaskState::parse(s)
                .ok_or_else(|| serde::de::Error::custom(format!("unknown task state: {s}"))),
            Value::Number(n) => Ok(match n.as_i64() {
                Some(1) => TaskState::Submitted,
                Some(2) => TaskState::Working,
                Some(3) => TaskState::Completed,
                Some(4) => TaskState::Failed,
                Some(5) => TaskState::Canceled,
                Some(6) => TaskState::InputRequired,
                Some(7) => TaskState::Rejected,
                Some(8) => TaskState::AuthRequired,
                _ => TaskState::Unspecified,
            }),
            _ => Err(serde::de::Error::custom("task state must be a string")),
        }
    }
}

// ---------------------------------------------------------------------------
// Part (field-presence union)
// ---------------------------------------------------------------------------

/// A content part of a message or artifact. Exactly one of `text`, `raw`,
/// `url`, `data` is present on the wire.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub content: PartContent,
    /// Optional file name (file parts).
    pub filename: Option<String>,
    /// Media (MIME) type of the content.
    pub media_type: Option<String>,
    /// Extension metadata (e.g. `distri/part_type`).
    pub metadata: Option<Metadata>,
}

/// The content of a [`Part`].
#[derive(Debug, Clone, PartialEq)]
pub enum PartContent {
    Text(String),
    /// File bytes; base64 on the wire.
    Raw(Vec<u8>),
    /// File by URL.
    Url(String),
    /// Structured JSON.
    Data(Value),
}

impl Part {
    fn with_content(content: PartContent) -> Self {
        Part {
            content,
            filename: None,
            media_type: None,
            metadata: None,
        }
    }
    pub fn text(text: impl Into<String>) -> Self {
        Self::with_content(PartContent::Text(text.into()))
    }
    pub fn raw(bytes: Vec<u8>) -> Self {
        Self::with_content(PartContent::Raw(bytes))
    }
    pub fn url(url: impl Into<String>) -> Self {
        Self::with_content(PartContent::Url(url.into()))
    }
    pub fn data(value: Value) -> Self {
        Self::with_content(PartContent::Data(value))
    }
    pub fn with_media_type(mut self, media_type: impl Into<String>) -> Self {
        self.media_type = Some(media_type.into());
        self
    }
    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }
    /// Set one metadata key.
    pub fn with_metadata(mut self, key: impl Into<String>, value: Value) -> Self {
        self.metadata
            .get_or_insert_with(Metadata::new)
            .insert(key.into(), value);
        self
    }
    pub fn as_text(&self) -> Option<&str> {
        match &self.content {
            PartContent::Text(t) => Some(t),
            _ => None,
        }
    }
    pub fn as_data(&self) -> Option<&Value> {
        match &self.content {
            PartContent::Data(d) => Some(d),
            _ => None,
        }
    }
    /// A metadata value by key.
    pub fn metadata_value(&self, key: &str) -> Option<&Value> {
        self.metadata.as_ref().and_then(|m| m.get(key))
    }
}

impl Serialize for Part {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        match &self.content {
            PartContent::Text(t) => map.serialize_entry("text", t)?,
            PartContent::Raw(r) => map.serialize_entry("raw", &BASE64.encode(r))?,
            PartContent::Url(u) => map.serialize_entry("url", u)?,
            PartContent::Data(d) => map.serialize_entry("data", d)?,
        }
        if let Some(f) = &self.filename {
            map.serialize_entry("filename", f)?;
        }
        if let Some(m) = &self.media_type {
            map.serialize_entry("mediaType", m)?;
        }
        if let Some(meta) = self.metadata.as_ref().filter(|m| !m.is_empty()) {
            map.serialize_entry("metadata", meta)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Part {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Metadata::deserialize(deserializer)?;
        let str_of = |k: &str| raw.get(k).and_then(Value::as_str).map(String::from);
        let mut filename = str_of("filename");
        let mut media_type = str_of("mediaType").or_else(|| str_of("media_type"));
        let content = if let Some(t) = str_of("text") {
            PartContent::Text(t)
        } else if let Some(r) = str_of("raw") {
            PartContent::Raw(BASE64.decode(r).map_err(serde::de::Error::custom)?)
        } else if let Some(u) = str_of("url") {
            PartContent::Url(u)
        } else if let Some(d) = raw.get("data") {
            PartContent::Data(d.clone())
        } else if let Some(Value::Object(file)) = raw.get("file") {
            // 0.x `{kind: "file", file: {uri | bytes, mimeType, name}}`.
            let fstr = |k: &str| file.get(k).and_then(Value::as_str).map(String::from);
            if filename.is_none() {
                filename = fstr("name");
            }
            if media_type.is_none() {
                media_type = fstr("mimeType");
            }
            if let Some(uri) = fstr("uri") {
                PartContent::Url(uri)
            } else if let Some(bytes) = fstr("bytes") {
                PartContent::Raw(BASE64.decode(bytes).map_err(serde::de::Error::custom)?)
            } else {
                return Err(serde::de::Error::custom("file part needs uri or bytes"));
            }
        } else {
            return Err(serde::de::Error::custom(
                "Part must have one of: text, raw, url, data",
            ));
        };
        let metadata = match raw.get("metadata") {
            Some(Value::Object(m)) => Some(m.clone()),
            _ => None,
        };
        Ok(Part {
            content,
            filename,
            media_type,
            metadata,
        })
    }
}

// ---------------------------------------------------------------------------
// Timestamps: ISO-8601 out; ISO-8601 or epoch milliseconds in (0.x).
// ---------------------------------------------------------------------------

/// Parse an ISO-8601 timestamp, or the 0.x epoch-milliseconds string/number.
pub fn parse_timestamp(v: &Value) -> Option<DateTime<Utc>> {
    match v {
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&Utc))
            .ok()
            .or_else(|| {
                s.parse::<i64>()
                    .ok()
                    .and_then(DateTime::from_timestamp_millis)
            }),
        Value::Number(n) => n.as_i64().and_then(DateTime::from_timestamp_millis),
        _ => None,
    }
}

/// ISO-8601 (RFC 3339, UTC, millisecond precision) — the wire format.
pub fn format_timestamp(t: &DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

mod ts_serde {
    use super::*;
    pub fn serialize<S: Serializer>(t: &Option<DateTime<Utc>>, s: S) -> Result<S::Ok, S::Error> {
        match t {
            Some(t) => s.serialize_str(&format_timestamp(t)),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DateTime<Utc>>, D::Error> {
        let v = Option::<Value>::deserialize(d)?;
        Ok(v.as_ref().and_then(parse_timestamp))
    }
}

// ---------------------------------------------------------------------------
// Message
// ---------------------------------------------------------------------------

/// A single message between user and agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub role: Role,
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    /// Extension URIs this message uses (activates them for the request).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_task_ids: Vec<String>,
}

impl Message {
    /// A message with a fresh id.
    pub fn new(role: Role, parts: Vec<Part>) -> Self {
        Message {
            message_id: uuid::Uuid::new_v4().to_string(),
            role,
            parts,
            ..Default::default()
        }
    }
    /// Text of all text parts joined by a space, if any.
    pub fn text(&self) -> Option<String> {
        let text: Vec<&str> = self.parts.iter().filter_map(Part::as_text).collect();
        (!text.is_empty()).then(|| text.join(" "))
    }
}

// ---------------------------------------------------------------------------
// Task / TaskStatus / Artifact
// ---------------------------------------------------------------------------

/// A task's status at a point in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub state: TaskState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    /// ISO-8601 on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "ts_serde")]
    pub timestamp: Option<DateTime<Utc>>,
}

impl TaskStatus {
    pub fn new(state: TaskState) -> Self {
        TaskStatus {
            state,
            message: None,
            timestamp: Some(Utc::now()),
        }
    }
}

/// A stateful operation between client and agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub context_id: String,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

/// A resource the agent produced during a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub artifact_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
}

// ---------------------------------------------------------------------------
// Streaming events
// ---------------------------------------------------------------------------

/// A task's status changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatusUpdateEvent {
    pub task_id: String,
    pub context_id: String,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

/// An artifact was produced or extended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TaskArtifactUpdateEvent {
    pub task_id: String,
    pub context_id: String,
    pub artifact: Artifact,
    /// Append these parts to the artifact sent earlier with the same id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub append: Option<bool>,
    /// The artifact is complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chunk: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

/// One streamed event (`SendStreamingMessage`, `SubscribeToTask`):
/// `{"task"|"message"|"statusUpdate"|"artifactUpdate": ...}`.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum StreamResponse {
    Task(Task),
    Message(Message),
    StatusUpdate(TaskStatusUpdateEvent),
    ArtifactUpdate(TaskArtifactUpdateEvent),
}

impl Serialize for StreamResponse {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            StreamResponse::Task(t) => map.serialize_entry("task", t)?,
            StreamResponse::Message(m) => map.serialize_entry("message", m)?,
            StreamResponse::StatusUpdate(s) => map.serialize_entry("statusUpdate", s)?,
            StreamResponse::ArtifactUpdate(a) => map.serialize_entry("artifactUpdate", a)?,
        }
        map.end()
    }
}

fn from_value<T: serde::de::DeserializeOwned, E: serde::de::Error>(v: Value) -> Result<T, E> {
    serde_json::from_value(v).map_err(E::custom)
}

impl<'de> Deserialize<'de> for StreamResponse {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut raw = Metadata::deserialize(deserializer)?;
        if let Some(v) = raw.remove("statusUpdate") {
            return Ok(StreamResponse::StatusUpdate(from_value(v)?));
        }
        if let Some(v) = raw.remove("artifactUpdate") {
            return Ok(StreamResponse::ArtifactUpdate(from_value(v)?));
        }
        if let Some(v) = raw.remove("message") {
            return Ok(StreamResponse::Message(from_value(v)?));
        }
        if let Some(v) = raw.remove("task") {
            return Ok(StreamResponse::Task(from_value(v)?));
        }
        // 0.x: the object itself, tagged with `kind`.
        let kind = raw.get("kind").and_then(Value::as_str).map(String::from);
        let v = Value::Object(raw);
        match kind.as_deref() {
            Some("status-update") => Ok(StreamResponse::StatusUpdate(from_value(v)?)),
            Some("artifact-update") => Ok(StreamResponse::ArtifactUpdate(from_value(v)?)),
            Some("message") => Ok(StreamResponse::Message(from_value(v)?)),
            Some("task") => Ok(StreamResponse::Task(from_value(v)?)),
            _ => Err(serde::de::Error::custom("unknown StreamResponse variant")),
        }
    }
}

/// The `SendMessage` result: `{"task": ...}` or `{"message": ...}`.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum SendMessageResponse {
    Task(Task),
    Message(Message),
}

impl Serialize for SendMessageResponse {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            SendMessageResponse::Task(t) => map.serialize_entry("task", t)?,
            SendMessageResponse::Message(m) => map.serialize_entry("message", m)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for SendMessageResponse {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut raw = Metadata::deserialize(deserializer)?;
        if let Some(v) = raw.remove("task") {
            return Ok(SendMessageResponse::Task(from_value(v)?));
        }
        if let Some(v) = raw.remove("message") {
            return Ok(SendMessageResponse::Message(from_value(v)?));
        }
        // 0.x: a bare Task or Message.
        let v = Value::Object(raw);
        if v.get("status").is_some() {
            Ok(SendMessageResponse::Task(from_value(v)?))
        } else {
            Ok(SendMessageResponse::Message(from_value(v)?))
        }
    }
}

// ---------------------------------------------------------------------------
// Request params
// ---------------------------------------------------------------------------

/// Push-notification target (`PushNotificationConfig`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PushNotificationConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentication: Option<AuthenticationInfo>,
}

/// Credentials for a push-notification endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationInfo {
    pub scheme: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<String>,
}

/// A push config bound to a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TaskPushNotificationConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default)]
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_notification_config: Option<PushNotificationConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

/// Options for `SendMessage` / `SendStreamingMessage`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageConfiguration {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_output_modes: Vec<String>,
    /// Wait for a terminal/interrupted state before answering (default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_length: Option<i32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "taskPushNotificationConfig"
    )]
    pub push_notification_config: Option<PushNotificationConfig>,
    /// a2a-rs spelling of `blocking: false`; read on input only.
    #[serde(default, skip_serializing)]
    pub return_immediately: Option<bool>,
}

/// Params of `SendMessage` / `SendStreamingMessage`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageRequest {
    pub message: Message,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<SendMessageConfiguration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

/// 0.x name of [`SendMessageRequest`].
pub type MessageSendParams = SendMessageRequest;
/// 0.x name of [`SendMessageConfiguration`].
pub type MessageSendConfiguration = SendMessageConfiguration;

/// Params of `GetTask`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GetTaskRequest {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_length: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

/// Params of `CancelTask` / `SubscribeToTask` (and 0.x `tasks/*`): a task id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TaskIdParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

/// Params of `ListTasks`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_size: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_length: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "ts_serde")]
    pub status_timestamp_after: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_artifacts: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

/// Result of `ListTasks`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksResponse {
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub next_page_token: String,
    #[serde(default)]
    pub page_size: i32,
    #[serde(default)]
    pub total_size: i32,
}
