use actix_web::{HttpResponse, web};
use distri_types::stores::{
    ProviderStore, TestProviderRequest, TestProviderResponse, UpsertProviderRequest,
};
use distri_types::{ModelProvider, SecretKeyDefinition, models::Model};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

pub fn configure_provider_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/providers")
            .route(web::get().to(list_providers))
            .route(web::post().to(upsert_provider)),
    )
    .service(web::resource("/providers/default-model").route(web::get().to(get_default_model)))
    // `/providers/test` must be registered before `/providers/{provider_id}`
    // so the literal segment is matched first.
    .service(web::resource("/providers/test").route(web::post().to(test_provider)))
    .service(web::resource("/providers/{provider_id}").route(web::delete().to(delete_provider)));
}

#[derive(Debug, Serialize)]
struct ModelProviderDefinitionResponse {
    id: String,
    label: String,
    keys: Vec<SecretKeyDefinition>,
    models: Vec<Model>,
    is_custom: bool,
}

#[utoipa::path(
    get,
    path = "/v1/providers",
    tag = "Providers",
    responses(
        (status = 200, description = "List providers with keys and models"),
    )
)]
async fn list_providers() -> HttpResponse {
    let defs = ModelProvider::all_provider_definitions();
    let model_groups = ModelProvider::well_known_models();

    let mut out = Vec::with_capacity(defs.len());
    for d in defs {
        let models = model_groups
            .iter()
            .find(|m| m.provider_id == d.id)
            .map(|m| m.models.clone())
            .unwrap_or_default();
        out.push(ModelProviderDefinitionResponse {
            id: d.id,
            label: d.label,
            keys: d.keys,
            models,
            is_custom: false,
        });
    }

    HttpResponse::Ok().json(out)
}

#[utoipa::path(
    post,
    path = "/v1/providers",
    tag = "Providers",
    request_body = UpsertProviderRequest,
    responses(
        (status = 200, description = "Provider upserted"),
        (status = 400, description = "Bad request"),
        (status = 500, description = "Internal server error")
    )
)]
async fn upsert_provider(
    store: web::Data<Arc<dyn ProviderStore>>,
    payload: web::Json<UpsertProviderRequest>,
) -> HttpResponse {
    // Validate secrets are non-empty before touching the store.
    for (key, value) in &payload.secrets {
        if key.is_empty() || value.is_empty() {
            return HttpResponse::BadRequest()
                .json(json!({"error": "Secret keys and values must be non-empty"}));
        }
    }

    match store.upsert_provider(payload.into_inner()).await {
        Ok(result) => HttpResponse::Ok().json(result),
        Err(err) => {
            tracing::error!(error = ?err, "Failed to upsert provider");
            HttpResponse::InternalServerError().json(json!({"error": "Failed to save provider"}))
        }
    }
}

#[utoipa::path(
    delete,
    path = "/v1/providers/{provider_id}",
    tag = "Providers",
    params(
        ("provider_id" = String, Path, description = "Provider ID"),
    ),
    responses(
        (status = 200, description = "Provider deleted"),
        (status = 500, description = "Internal server error")
    )
)]
async fn delete_provider(
    store: web::Data<Arc<dyn ProviderStore>>,
    path: web::Path<String>,
) -> HttpResponse {
    let provider_id = path.into_inner();
    match store.delete_provider(&provider_id).await {
        Ok(()) => HttpResponse::Ok().json(json!({"deleted": true})),
        Err(err) => {
            tracing::error!(error = ?err, provider = %provider_id, "Failed to delete provider");
            HttpResponse::InternalServerError().json(json!({"error": "Failed to delete provider"}))
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/providers/default-model",
    tag = "Providers",
    responses(
        (status = 200, description = "Default model retrieved"),
        (status = 500, description = "Internal server error")
    )
)]
async fn get_default_model(store: web::Data<Arc<dyn ProviderStore>>) -> HttpResponse {
    match store.get_default_model().await {
        Ok(default_model) => HttpResponse::Ok().json(json!({ "default_model": default_model })),
        Err(err) => {
            tracing::error!(error = ?err, "Failed to get default model");
            HttpResponse::InternalServerError()
                .json(json!({"error": "Failed to get default model"}))
        }
    }
}

/// Minimal shape of an OpenAI-style `GET /models` response.
#[derive(Debug, Deserialize)]
struct ModelsListResponse {
    #[serde(default)]
    data: Vec<ModelsListEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelsListEntry {
    id: String,
}

/// Probe `GET {base_url}/models` to validate a URL + key combination.
/// Sends the key as both `Authorization: Bearer` (OpenAI) and `api-key`
/// (Azure) so one request works for any OpenAI-compatible endpoint.
async fn probe_models(base_url: &str, api_key: &str) -> Result<Vec<String>, String> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("api-key", api_key)
        .send()
        .await
        .map_err(|e| format!("request to {url} failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let detail: String = body.chars().take(300).collect();
        return Err(format!("endpoint returned {status}: {detail}"));
    }

    let parsed: ModelsListResponse = resp
        .json()
        .await
        .map_err(|e| format!("invalid /models response: {e}"))?;
    Ok(parsed.data.into_iter().map(|m| m.id).collect())
}

/// Probe using a catalog-supplied [`ProviderTestConfig`] — used by providers
/// like fal.ai that don't expose a `GET /models` listing.
///
/// Sends the configured request (method, body, auth header style) and
/// treats any response outside the fail set as success: by default
/// **any status other than 401/403** counts as "the auth header reached
/// the server," which is enough to confirm the key is valid. A provider
/// can narrow this via `accept_status`.
async fn probe_with_config(
    cfg: &distri_types::ProviderTestConfig,
    base_url: &str,
    api_key: &str,
) -> Result<(), String> {
    let url = cfg.url.replace("{base_url}", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let mut req = match cfg.method.to_uppercase().as_str() {
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        _ => client.get(&url),
    };
    req = match cfg.auth.to_lowercase().as_str() {
        "key" => req.header("Authorization", format!("Key {api_key}")),
        "api_key" | "api-key" => req.header("api-key", api_key),
        _ => req.header("Authorization", format!("Bearer {api_key}")),
    };
    if let Some(body) = &cfg.body {
        req = req.json(body);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("request to {url} failed: {e}"))?;
    let status = resp.status().as_u16();

    let ok = if !cfg.accept_status.is_empty() {
        cfg.accept_status.contains(&status)
    } else {
        // Default: anything outside 401/403 means the auth header reached the
        // server — a 4xx validation error still proves the key is valid.
        status != 401 && status != 403
    };
    if ok {
        return Ok(());
    }
    let body = resp.text().await.unwrap_or_default();
    let detail: String = body.chars().take(300).collect();
    Err(format!("endpoint returned {status}: {detail}"))
}

#[utoipa::path(
    post,
    path = "/v1/providers/test",
    tag = "Providers",
    request_body = TestProviderRequest,
    responses(
        (status = 200, description = "Test result", body = TestProviderResponse),
        (status = 400, description = "Bad request"),
    )
)]
async fn test_provider(
    store: web::Data<Arc<dyn ProviderStore>>,
    payload: web::Json<TestProviderRequest>,
) -> HttpResponse {
    let provider_id = payload.into_inner().provider_id;

    // The store resolves the endpoint URL + API key from stored config —
    // the caller never sends credentials.
    let endpoint = match store.resolve_provider_endpoint(&provider_id).await {
        Ok(ep) => ep,
        Err(err) => {
            return HttpResponse::Ok().json(TestProviderResponse {
                ok: false,
                models: vec![],
                error: Some(format!("{err}")),
            });
        }
    };

    // Per-provider override: providers without a `/models` endpoint (fal.ai)
    // ship a `test` block in their catalog file. When present we use it;
    // otherwise we fall back to the OpenAI-style `/models` probe.
    let result = if let Some(cfg) = distri_types::lookup_provider_test_config(&provider_id) {
        match probe_with_config(&cfg, &endpoint.base_url, &endpoint.api_key).await {
            Ok(()) => {
                // The override probe doesn't list models — surface the
                // catalog's own model ids so the UI can show them.
                let models = ModelProvider::well_known_models()
                    .into_iter()
                    .find(|m| m.provider_id == provider_id)
                    .map(|m| m.models.into_iter().map(|m| m.id).collect())
                    .unwrap_or_default();
                TestProviderResponse {
                    ok: true,
                    models,
                    error: None,
                }
            }
            Err(error) => TestProviderResponse {
                ok: false,
                models: vec![],
                error: Some(error),
            },
        }
    } else {
        match probe_models(&endpoint.base_url, &endpoint.api_key).await {
            Ok(models) => TestProviderResponse {
                ok: true,
                models,
                error: None,
            },
            Err(error) => TestProviderResponse {
                ok: false,
                models: vec![],
                error: Some(error),
            },
        }
    };
    HttpResponse::Ok().json(result)
}
