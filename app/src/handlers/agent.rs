use crate::{
    db::DbPool,
    middleware::auth::AuthSession,
    models::agent::{Agent, WrappedDefinition},
};
use actix_web::{get, post, web, HttpResponse, Result};
use agents::AgentDefinition;
use diesel::prelude::*;

#[get("")]
pub async fn list_agents(
    pool: web::Data<DbPool>,
    auth_session: web::ReqData<AuthSession>,
) -> Result<HttpResponse> {
    use crate::schema::agents::dsl::*;

    let mut conn = pool.get().expect("Failed to get DB connection");
    let session_user_id = auth_session.session.user_id;

    let results = web::block(move || {
        agents
            .filter(user_id.eq(session_user_id))
            .select(Agent::as_select())
            .load(&mut conn)
    })
    .await?
    .map_err(|e| {
        eprintln!("Error loading agents: {}", e);
        actix_web::error::ErrorInternalServerError("Error loading agents")
    })?;

    Ok(HttpResponse::Ok().json(results))
}

#[post("")]
pub async fn create_agent(
    pool: web::Data<DbPool>,
    auth_session: web::ReqData<AuthSession>,
    agent_data: web::Json<AgentDefinition>,
) -> Result<HttpResponse> {
    use crate::schema::agents::dsl::*;

    let def_clone = agent_data.into_inner().clone();
    let mut conn = pool.get().expect("Failed to get DB connection");
    let session = &auth_session.session;
    let def = WrappedDefinition {
        definition: def_clone.clone(),
    };
    let session_user_id = session.user_id;
    let new_agent = web::block(move || {
        diesel::insert_into(agents)
            .values((
                name.eq(&def_clone.name),
                description.eq(&def_clone.description),
                definition.eq(&def),
                user_id.eq(session_user_id),
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
