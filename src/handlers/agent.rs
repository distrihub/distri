use crate::{
    db::DbPool,
    models::agent::{Agent, CreateAgentRequest},
    models::user::User,
};
use actix_web::{get, post, web, HttpResponse, Result};
use diesel::prelude::*;

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
    // user: web::ReqData<User>,
) -> Result<HttpResponse> {
    use crate::schema::agents::dsl::*;

    let mut conn = pool.get().expect("Failed to get DB connection");

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
                // user_id.eq(user.id),
                user_id.eq(Some(1)),
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
