use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable)]
#[diesel(table_name = crate::schema::agents)]
pub struct Agent {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub tools: Option<Value>,
    pub model: String,
    pub model_settings: Option<Value>,
    pub provider_name: String,
    pub prompt: Option<String>,
    pub avatar: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub user_id: Option<i32>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentRequest {
    pub name: String,
    pub description: Option<String>,
    pub tools: Option<Value>,
    pub model: String,
    pub model_settings: Option<Value>,
    pub provider_name: String,
    pub prompt: Option<String>,
    pub avatar: Option<String>,
    pub tags: Option<Vec<String>>,
}
