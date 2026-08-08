use actix_web::{web, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_core::secrets::SecretResolver;
use distri_types::{ModelProvider, ModelWithProvider};
use std::sync::Arc;

pub fn configure_model_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/models").route(web::get().to(list_models)));
}

#[utoipa::path(
    get,
    path = "/v1/models",
    tag = "Models",
    responses(
        (status = 200, description = "List all models, denormalized with provider info")
    )
)]
/// Returns all supported models as a flat list, each row denormalized with its
/// provider's id/label and a `configured: bool` indicating whether the
/// provider's required API key(s) are set in secrets or environment.
///
/// This is the shared `GET /v1/models` contract — it must match the shape the
/// `distri` client (`Vec<ModelWithProvider>`) and distri-cloud emit.
async fn list_models(executor: web::Data<Arc<AgentOrchestrator>>) -> HttpResponse {
    let secret_store = executor.stores.secret_store.clone();
    let resolver = SecretResolver::new(secret_store);

    let provider_models = ModelProvider::well_known_models();
    let provider_defs = ModelProvider::all_provider_definitions();

    let mut result: Vec<ModelWithProvider> = Vec::new();

    for pm in provider_models {
        // Find the provider definition to get required keys
        let configured = if let Some(def) = provider_defs.iter().find(|d| d.id == pm.provider_id) {
            // A provider is configured if ALL its required keys are present
            let mut all_present = true;
            for key_def in &def.keys {
                if key_def.required && resolver.resolve(&key_def.key).await.is_none() {
                    all_present = false;
                    break;
                }
            }
            all_present
        } else {
            // No key requirements — always configured
            true
        };

        for model in pm.models {
            result.push(ModelWithProvider {
                model,
                provider_id: pm.provider_id.clone(),
                provider_label: pm.provider_label.clone(),
                configured,
            });
        }
    }

    HttpResponse::Ok().json(result)
}
