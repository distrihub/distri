use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable, Selectable)]
#[diesel(table_name = crate::schema::agents)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Agent {
    pub id: i32,
    pub name: String,
    pub description: String,
    pub definition: Value,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub user_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentRequest {
    pub name: String,
    pub description: String,
    pub definition: Value,
}
