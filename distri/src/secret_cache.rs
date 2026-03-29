use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::RwLock;

use crate::Distri;

struct CachedEntry {
    value: String,
    expires_at: Instant,
}

/// Client-side cache for resolved secrets and connection tokens.
///
/// Resolution priority: env_vars > cache (non-expired) > batch fetch via server.
pub struct SecretCache {
    client: Arc<Distri>,
    cache: Arc<RwLock<HashMap<String, CachedEntry>>>,
    default_ttl: Duration,
}

impl SecretCache {
    pub fn new(client: Arc<Distri>) -> Self {
        Self {
            client,
            cache: Arc::new(RwLock::new(HashMap::new())),
            default_ttl: Duration::from_secs(300), // 5 minutes
        }
    }

    /// Resolve variable names to their values.
    ///
    /// Priority: 1) env_vars, 2) cache hit (non-expired), 3) batch fetch via server.
    pub async fn resolve_vars(
        &self,
        var_names: &[String],
        env_vars: &HashMap<String, String>,
    ) -> Result<HashMap<String, String>> {
        let mut resolved = HashMap::new();
        let mut to_fetch = Vec::new();

        // 1) Check env_vars first
        // 2) Then check cache
        {
            let cache = self.cache.read().await;
            let now = Instant::now();
            for name in var_names {
                if let Some(val) = env_vars.get(name) {
                    resolved.insert(name.clone(), val.clone());
                } else if let Some(entry) = cache.get(name) {
                    if entry.expires_at > now {
                        resolved.insert(name.clone(), entry.value.clone());
                    } else {
                        to_fetch.push(name.clone());
                    }
                } else {
                    to_fetch.push(name.clone());
                }
            }
        }

        // 3) Batch fetch remaining from server
        if !to_fetch.is_empty() {
            let keys: Vec<&str> = to_fetch.iter().map(|s| s.as_str()).collect();
            match self.client.resolve_secrets(&keys).await {
                Ok(fetched) => {
                    let mut cache = self.cache.write().await;
                    let expires_at = Instant::now() + self.default_ttl;
                    for (k, v) in &fetched {
                        cache.insert(
                            k.clone(),
                            CachedEntry {
                                value: v.clone(),
                                expires_at,
                            },
                        );
                        resolved.insert(k.clone(), v.clone());
                    }
                }
                Err(e) => {
                    tracing::warn!("failed to resolve secrets: {}", e);
                    // Continue with what we have — unresolved vars stay as $VAR_NAME
                }
            }
        }

        Ok(resolved)
    }

    /// Resolve a connection token by connection ID.
    ///
    /// Returns the access token string. Caches using `expires_at` from the
    /// response when available, otherwise uses the default TTL.
    pub async fn resolve_connection_token(&self, connection_id: &str) -> Result<String> {
        let cache_key = format!("__connection:{}", connection_id);

        // Check cache
        {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(&cache_key) {
                if entry.expires_at > Instant::now() {
                    return Ok(entry.value.clone());
                }
            }
        }

        // Fetch from server
        let token = self.client.get_connection_token(connection_id).await?;

        // Determine expiry
        let expires_at = if let Some(ref expires_str) = token.expires_at {
            // Parse ISO 8601 and compute duration from now
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(expires_str) {
                let now = chrono::Utc::now();
                let dur = (dt.with_timezone(&chrono::Utc) - now)
                    .to_std()
                    .unwrap_or(self.default_ttl);
                // Subtract a small buffer so we refresh before actual expiry
                Instant::now() + dur.saturating_sub(Duration::from_secs(30))
            } else {
                Instant::now() + self.default_ttl
            }
        } else {
            Instant::now() + self.default_ttl
        };

        // Cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(
                cache_key,
                CachedEntry {
                    value: token.access_token.clone(),
                    expires_at,
                },
            );
        }

        Ok(token.access_token)
    }

    /// Remove a specific entry from the cache.
    pub async fn invalidate(&self, key: &str) {
        self.cache.write().await.remove(key);
    }

    /// Clear all cached entries.
    pub async fn clear(&self) {
        self.cache.write().await.clear();
    }
}
