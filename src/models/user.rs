use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable, Clone)]
#[diesel(table_name = crate::schema::users)]
pub struct User {
    pub id: i32,
    pub firebase_id: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub firebase_id: String,
}
