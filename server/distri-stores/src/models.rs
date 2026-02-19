use crate::schema::*;
use chrono::NaiveDateTime;
use diesel::prelude::*;

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = agent_configs, primary_key(name))]
pub struct AgentConfigModel {
    pub name: String,
    pub config: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = agent_configs)]
pub struct NewAgentConfigModel<'a> {
    pub name: &'a str,
    pub config: &'a str,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = agent_configs)]
pub struct AgentConfigChangeset<'a> {
    pub config: &'a str,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = threads)]
pub struct ThreadModel {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub message_count: i32,
    pub last_message: Option<String>,
    pub metadata: String,
    pub attributes: String,
    pub external_id: Option<String>,
    pub user_id: String,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = threads)]
pub struct NewThreadModel<'a> {
    pub id: &'a str,
    pub agent_id: &'a str,
    pub title: &'a str,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub message_count: i32,
    pub last_message: Option<&'a str>,
    pub metadata: &'a str,
    pub attributes: &'a str,
    pub external_id: Option<&'a str>,
    pub user_id: &'a str,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = threads)]
pub struct ThreadChangeset<'a> {
    pub title: Option<&'a str>,
    pub updated_at: NaiveDateTime,
    pub message_count: Option<i32>,
    pub last_message: Option<Option<&'a str>>,
    pub metadata: Option<&'a str>,
    pub attributes: Option<&'a str>,
    pub external_id: Option<Option<&'a str>>,
}

#[derive(
    Debug, Clone, Queryable, Identifiable, Selectable, Associations, Insertable, AsChangeset,
)]
#[diesel(table_name = tasks)]
#[diesel(belongs_to(ThreadModel, foreign_key = thread_id))]
pub struct TaskModel {
    pub id: String,
    pub thread_id: String,
    pub parent_task_id: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = tasks)]
pub struct NewTaskModel<'a> {
    pub id: &'a str,
    pub thread_id: &'a str,
    pub parent_task_id: Option<&'a str>,
    pub status: &'a str,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = tasks)]
pub struct TaskStatusChangeset<'a> {
    pub status: &'a str,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Queryable, Identifiable, Associations, Selectable)]
#[diesel(table_name = task_messages)]
#[diesel(belongs_to(TaskModel, foreign_key = task_id))]
pub struct TaskMessageModel {
    pub id: i32,
    pub task_id: String,
    pub kind: String,
    pub payload: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = task_messages)]
pub struct NewTaskMessageModel<'a> {
    pub task_id: &'a str,
    pub kind: &'a str,
    pub payload: &'a str,
    pub created_at: i64,
}

#[derive(Debug, Clone, Queryable, Identifiable, Selectable, AsChangeset)]
#[diesel(table_name = session_entries)]
#[diesel(primary_key(thread_id, key))]
pub struct SessionEntryModel {
    pub thread_id: String,
    pub key: String,
    pub value: String,
    pub expiry: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = session_entries)]
pub struct NewSessionEntryModel<'a> {
    pub thread_id: &'a str,
    pub key: &'a str,
    pub value: &'a str,
    pub expiry: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = session_entries)]
pub struct SessionEntryChangeset<'a> {
    pub value: &'a str,
    pub expiry: Option<NaiveDateTime>,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = memory_entries)]
pub struct MemoryEntryModel {
    pub id: i32,
    pub user_id: String,
    pub content: String,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = memory_entries)]
pub struct NewMemoryEntryModel<'a> {
    pub user_id: &'a str,
    pub content: &'a str,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = scratchpad_entries)]
pub struct ScratchpadEntryModel {
    pub id: i32,
    pub thread_id: String,
    pub task_id: String,
    pub parent_task_id: Option<String>,
    pub entry: String,
    pub entry_type: Option<String>,
    pub timestamp: i64,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = scratchpad_entries)]
pub struct NewScratchpadEntryModel<'a> {
    pub thread_id: &'a str,
    pub task_id: &'a str,
    pub parent_task_id: Option<&'a str>,
    pub entry: &'a str,
    pub entry_type: Option<&'a str>,
    pub timestamp: i64,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Identifiable)]
#[diesel(table_name = browser_sessions, primary_key(user_id))]
pub struct BrowserSessionModel {
    pub user_id: String,
    pub state: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = browser_sessions)]
pub struct NewBrowserSessionModel<'a> {
    pub user_id: &'a str,
    pub state: &'a str,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = browser_sessions)]
pub struct BrowserSessionChangeset<'a> {
    pub state: &'a str,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = integrations)]
pub struct IntegrationModel {
    pub id: String,
    pub user_id: String,
    pub provider: String,
    pub session_data: Option<String>,
    pub secrets_data: Option<String>,
    pub oauth_state: Option<String>,
    pub oauth_state_data: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = integrations)]
pub struct NewIntegrationModel<'a> {
    pub id: &'a str,
    pub user_id: &'a str,
    pub provider: &'a str,
    pub session_data: Option<&'a str>,
    pub secrets_data: Option<&'a str>,
    pub oauth_state: Option<&'a str>,
    pub oauth_state_data: Option<&'a str>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = integrations)]
pub struct IntegrationChangeset<'a> {
    pub session_data: Option<&'a str>,
    pub secrets_data: Option<&'a str>,
    pub oauth_state: Option<&'a str>,
    pub oauth_state_data: Option<&'a str>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = external_tool_calls)]
pub struct ExternalToolCallModel {
    pub id: String,
    pub status: String,
    pub request: Option<String>,
    pub response: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub locked_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = external_tool_calls)]
pub struct NewExternalToolCallModel<'a> {
    pub id: &'a str,
    pub status: &'a str,
    pub request: Option<&'a str>,
    pub response: Option<&'a str>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub locked_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = external_tool_calls)]
pub struct ExternalToolCallChangeset<'a> {
    pub status: Option<&'a str>,
    pub request: Option<Option<&'a str>>,
    pub response: Option<Option<&'a str>>,
    pub updated_at: NaiveDateTime,
    pub locked_at: Option<Option<NaiveDateTime>>,
}

#[derive(Debug, Clone, Queryable, Identifiable, Associations, Selectable)]
#[diesel(table_name = external_tool_call_events)]
#[diesel(belongs_to(ExternalToolCallModel, foreign_key = tool_call_id))]
pub struct ExternalToolCallEventModel {
    pub id: i32,
    pub tool_call_id: String,
    pub payload: String,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = external_tool_call_events)]
pub struct NewExternalToolCallEventModel<'a> {
    pub tool_call_id: &'a str,
    pub payload: &'a str,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = plugin_catalog, primary_key(package_name))]
pub struct PluginCatalogModel {
    pub package_name: String,
    pub version: Option<String>,
    pub object_prefix: String,
    pub entrypoint: Option<String>,
    pub artifact_json: String,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = plugin_catalog)]
pub struct NewPluginCatalogModel<'a> {
    pub package_name: &'a str,
    pub version: Option<&'a str>,
    pub object_prefix: &'a str,
    pub entrypoint: Option<&'a str>,
    pub artifact_json: &'a str,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = plugin_catalog)]
pub struct PluginCatalogChangeset<'a> {
    pub version: Option<&'a str>,
    pub object_prefix: &'a str,
    pub entrypoint: Option<&'a str>,
    pub artifact_json: &'a str,
    pub updated_at: NaiveDateTime,
}

// ========== Prompt Templates ==========

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = prompt_templates)]
pub struct PromptTemplateModel {
    pub id: String,
    pub name: String,
    pub template: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub is_system: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = prompt_templates)]
pub struct NewPromptTemplateModel<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub template: &'a str,
    pub description: Option<&'a str>,
    pub version: Option<&'a str>,
    pub is_system: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = prompt_templates)]
pub struct PromptTemplateChangeset<'a> {
    pub name: &'a str,
    pub template: &'a str,
    pub description: Option<&'a str>,
    pub version: Option<&'a str>,
    pub updated_at: NaiveDateTime,
}

// ========== Server Settings ==========

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = server_settings)]
pub struct ServerSettingsModel {
    pub id: String,
    pub config_json: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = server_settings)]
pub struct NewServerSettingsModel<'a> {
    pub id: &'a str,
    pub config_json: &'a str,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = server_settings)]
pub struct ServerSettingsChangeset<'a> {
    pub config_json: &'a str,
    pub updated_at: NaiveDateTime,
}

// ========== Secrets ==========

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = secrets)]
pub struct SecretModel {
    pub id: String,
    pub key: String,
    pub value: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = secrets)]
pub struct NewSecretModel<'a> {
    pub id: &'a str,
    pub key: &'a str,
    pub value: &'a str,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = secrets)]
pub struct SecretChangeset<'a> {
    pub value: &'a str,
    pub updated_at: NaiveDateTime,
}

// ========== Skills ==========

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = skills)]
pub struct SkillModel {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub content: String,
    pub tags: String,
    pub is_public: i32,
    pub is_system: i32,
    pub star_count: i32,
    pub clone_count: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = skills)]
pub struct NewSkillModel<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub content: &'a str,
    pub tags: &'a str,
    pub is_public: i32,
    pub is_system: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = skill_scripts)]
pub struct SkillScriptModel {
    pub id: String,
    pub skill_id: String,
    pub name: String,
    pub description: Option<String>,
    pub code: String,
    pub language: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = skill_scripts)]
pub struct NewSkillScriptModel<'a> {
    pub id: &'a str,
    pub skill_id: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub code: &'a str,
    pub language: &'a str,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

// ========== Message Reads ==========

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = message_reads)]
pub struct MessageReadModel {
    pub id: String,
    pub thread_id: String,
    pub message_id: String,
    pub user_id: String,
    pub read_at: NaiveDateTime,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = message_reads)]
pub struct NewMessageReadModel<'a> {
    pub id: &'a str,
    pub thread_id: &'a str,
    pub message_id: &'a str,
    pub user_id: &'a str,
    pub read_at: NaiveDateTime,
    pub created_at: NaiveDateTime,
}

// ========== Message Votes ==========

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = message_votes)]
pub struct MessageVoteModel {
    pub id: String,
    pub thread_id: String,
    pub message_id: String,
    pub user_id: String,
    pub vote_type: String,
    pub comment: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = message_votes)]
pub struct NewMessageVoteModel<'a> {
    pub id: &'a str,
    pub thread_id: &'a str,
    pub message_id: &'a str,
    pub user_id: &'a str,
    pub vote_type: &'a str,
    pub comment: Option<&'a str>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = message_votes)]
pub struct MessageVoteChangeset<'a> {
    pub vote_type: &'a str,
    pub comment: Option<Option<&'a str>>,
    pub updated_at: NaiveDateTime,
}
