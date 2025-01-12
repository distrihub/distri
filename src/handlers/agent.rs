use crate::{
    db::DbPool,
    models::agent::{Agent, CreateAgentRequest},
    models::user::User,
};
use actix_web::{get, post, web, HttpResponse, Result};
use diesel::prelude::*;
use rand::seq::SliceRandom;

#[get("/agents")]
pub async fn list_agents(pool: web::Data<DbPool>) -> Result<HttpResponse> {
    use crate::schema::agents::dsl::*;

    let mut conn = pool.get().expect("Failed to get DB connection");
    let results = web::block(move || agents.load::<Agent>(&mut conn))
        .await?
        .map_err(|e| {
            eprintln!("Error loading agents: {}", e);
            actix_web::error::ErrorInternalServerError("Error loading agents")
        })?;

    Ok(HttpResponse::Ok().json(results))
}

#[post("/agents")]
pub async fn create_agent(
    pool: web::Data<DbPool>,
    agent_data: web::Json<CreateAgentRequest>,
) -> Result<HttpResponse> {
    use crate::schema::agents::dsl::*;
    use crate::schema::users::dsl as users_dsl;

    let mut conn = pool.get().expect("Failed to get DB connection");

    // TODO: Temporary
    // Get random user from the two fake users
    let fake_user_ids: Vec<i32> = users_dsl::users
        .select(users_dsl::id)
        .load::<i32>(&mut conn)
        .expect("Error loading users");

    let random_user_id = fake_user_ids
        .choose(&mut rand::thread_rng())
        .expect("No users found")
        .clone();

    let new_agent = web::block(move || {
        diesel::insert_into(agents)
            .values((
                name.eq(&agent_data.name),
                description.eq(&agent_data.description),
                tools.eq(&agent_data.tools),
                model.eq(&agent_data.model),
                model_settings.eq(&agent_data.model_settings),
                provider_name.eq(&agent_data.provider_name),
                prompt.eq(&agent_data.prompt),
                avatar.eq(&agent_data.avatar),
                user_id.eq(random_user_id),
                tags.eq(&agent_data.tags),
            ))
            .get_result::<Agent>(&mut conn)
    })
    .await?
    .map_err(|e| {
        eprintln!("Error creating agent: {}", e);
        actix_web::error::ErrorInternalServerError("Error creating agent")
    })?;

    Ok(HttpResponse::Created().json(new_agent))
}
