use std::collections::HashMap;
use std::sync::Arc;

use crate::tools::inject_env::TokenFetcher;
use distri_types::stores::SecretStore;

// Re-export pure resolution functions from distri-types
pub use distri_types::resolve::{
    extract_vars, extract_vars_from_value, substitute_string, substitute_value,
};

/// Context for resolving variables from multiple sources.
pub struct ResolveContext {
    pub env_vars: HashMap<String, String>,
    pub secret_store: Option<Arc<dyn SecretStore>>,
    pub token_fetcher: Option<TokenFetcher>,
}

/// Resolve variables from sources in priority order: env_vars first, then secret_store.
/// Returns an error if any variable cannot be resolved.
pub async fn resolve_all(
    var_names: &[String],
    ctx: &ResolveContext,
) -> Result<HashMap<String, String>, String> {
    let mut resolved = HashMap::new();

    for name in var_names {
        // 1. Check env_vars
        if let Some(val) = ctx.env_vars.get(name) {
            resolved.insert(name.clone(), val.clone());
            continue;
        }

        // 2. Check secret_store
        if let Some(ref store) = ctx.secret_store {
            if let Ok(Some(secret)) = store.get(name).await {
                resolved.insert(name.clone(), secret.value.clone());
                continue;
            }
        }

        return Err(format!("unresolved variable: ${}", name));
    }

    Ok(resolved)
}

/// Fetch an OAuth token via the TokenFetcher callback.
/// Returns `(provider_name, access_token)`.
pub async fn resolve_connection_token(
    connection_id: &str,
    ctx: &ResolveContext,
) -> Result<(String, String), String> {
    let fetcher = ctx
        .token_fetcher
        .as_ref()
        .ok_or_else(|| "no token fetcher configured".to_string())?;

    (fetcher)(connection_id.to_string()).await
}
