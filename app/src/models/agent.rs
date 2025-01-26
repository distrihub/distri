use agents::AgentDefinition;
use chrono::NaiveDateTime;
use diesel::deserialize::{FromSql, FromSqlRow};
use diesel::expression::AsExpression;
use diesel::prelude::*;
use diesel::serialize::{Output, ToSql};
use diesel::sql_types::Jsonb;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable, Selectable)]
#[diesel(table_name = crate::schema::agents)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Agent {
    pub id: i32,
    pub name: String,
    pub description: String,
    pub definition: WrappedDefinition,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub user_id: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, FromSqlRow, AsExpression)]
#[diesel(sql_type = Jsonb)]
pub struct WrappedDefinition {
    pub definition: AgentDefinition,
}

impl ToSql<Jsonb, diesel::pg::Pg> for WrappedDefinition {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, diesel::pg::Pg>) -> diesel::serialize::Result {
        serde_json::to_writer(out, self)?;
        Ok(diesel::serialize::IsNull::No)
    }
}

impl FromSql<Jsonb, diesel::pg::Pg> for WrappedDefinition {
    fn from_sql(bytes: diesel::pg::PgValue) -> diesel::deserialize::Result<Self> {
        serde_json::from_slice(bytes.as_bytes()).map_err(|e| e.into())
    }
}
