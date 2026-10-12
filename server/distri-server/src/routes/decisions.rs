//! `POST /v1/decisions` — typed decisions (TypeSafe System One wire format).
//!
//! Thin: decode the typed request, hand it to
//! [`distri_core::decision_service::DecisionService`], render the typed
//! response or the [`ApiError`] the service error maps to.

use std::sync::{Arc, LazyLock};

use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_core::decision_service::DecisionService;
use distri_types::decision::DecisionRequest;
use distri_types::stores::ProviderStore;
use distri_types::ApiError;

use crate::routes_catalog::Route;

/// One connection pool for every decision call.
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

pub fn configure_decision_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource(Route::Decisions.path()).route(web::post().to(decide)));
}

#[utoipa::path(
    post,
    path = "/v1/decisions",
    tag = "Decisions",
    responses(
        (status = 200, description = "Typed answers, one per question (TypeSafe /v1/systemone response)"),
        (status = 400, description = "Malformed request, unknown provider, or missing provider secret"),
        (status = 422, description = "The provider rejected the request"),
        (status = 502, description = "The provider failed or was unreachable"),
    )
)]
pub async fn decide(
    executor: web::Data<Arc<AgentOrchestrator>>,
    // The same workspace-scoped store `/v1/providers` uses: the standalone
    // server registers its settings-backed store, the cloud its `PgProviderStore`.
    provider_store: Option<web::Data<Arc<dyn ProviderStore>>>,
    payload: web::Json<DecisionRequest>,
) -> Result<HttpResponse, ApiError> {
    let secret_store = executor
        .stores
        .secret_store
        .clone()
        .ok_or_else(|| ApiError::ServiceUnavailable("secret store is not configured".into()))?;
    let service = DecisionService::new(
        HTTP_CLIENT.clone(),
        secret_store,
        provider_store.map(|store| store.get_ref().clone()),
    );

    let req = payload.into_inner();
    let res = service.decide(&req).await.inspect_err(|e| {
        tracing::warn!(model = %req.model, error = %e, "decision failed");
    })?;
    tracing::info!(
        model = %req.model,
        questions = req.questions.len(),
        input_tokens = res.usage.input_tokens,
        "decision answered"
    );
    Ok(HttpResponse::Ok().json(res))
}
