use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Queryable, Selectable)]
#[diesel(table_name = crate::schema::user_memory)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserMemory {
    pub id: i32,
    pub user_id: i32,
    pub memory: String,
    pub created_at: NaiveDateTime,
    pub valid_until: Option<NaiveDateTime>,
}

#[derive(Insertable)]
#[diesel(table_name = crate::schema::user_memory)]
pub struct NewUserMemory {
    pub user_id: i32,
    pub memory: String,
    pub valid_until: Option<NaiveDateTime>,
}
