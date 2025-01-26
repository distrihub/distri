use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable, Clone)]
#[diesel(table_name = crate::schema::users)]
pub struct User {
    pub id: i32,
    pub twitter_id: String,
    pub name: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub twitter_url: Option<String>,
    pub profile_image_url: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}
