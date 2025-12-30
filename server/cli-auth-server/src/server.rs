use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use actix_cors::Cors;
use actix_web::{web, App, HttpServer};
use distri_types::auth::{
    append_pkce_challenge, generate_pkce_pair, AuthError, AuthType, OAuth2State, OAuthHandler,
    ProviderRegistry, PKCE_CODE_VERIFIER_KEY,
};
use dotenvy::dotenv;
use rand::Rng;
use serde_json::Value;
use tracing::{info, warn};
use url::{Position, Url};

use crate::oauth_handler::{
    handle_oauth_callback, health_check, start_oauth_flow, OAuthHandlerState, PendingSession,
};

#[derive(Debug, Clone)]
pub struct CallbackConfig {
    pub bind_addr: SocketAddr,
    pub callback_base_url: String,
}

impl CallbackConfig {
    const ENV_KEYS: [&'static str; 2] = ["DISTRI_AUTH_CALLBACK_URL", "TWITTER_REDIRECT_URI"];

    pub fn from_env() -> Result<Option<Self>, AuthError> {
        let _ = dotenv();

        for key in Self::ENV_KEYS {
            match std::env::var(key) {
                Ok(value) if !value.trim().is_empty() => {
                    return Self::from_raw(&value).map(Some).map_err(|err| {
                        AuthError::InvalidConfig(format!(
                            "invalid callback url provided via {}: {}",
                            key, err
                        ))
                    });
                }
                Ok(_) => continue,
                Err(std::env::VarError::NotPresent) => continue,
                Err(std::env::VarError::NotUnicode(_)) => {
                    return Err(AuthError::InvalidConfig(format!(
                        "environment variable {} contains invalid unicode",
                        key
                    )))
                }
            }
        }

        Ok(None)
    }

    pub fn from_bind_addr(bind_addr: SocketAddr) -> Self {
        let callback_base_url = format!("http://{}", bind_addr);
        Self {
            bind_addr,
            callback_base_url,
        }
    }

    pub fn callback_url(&self) -> String {
        format!(
            "{}/auth/callback",
            self.callback_base_url.trim_end_matches('/')
        )
    }

    fn from_raw(raw: &str) -> Result<Self, String> {
        let url = Url::parse(raw).map_err(|e| format!("failed to parse url '{}': {}", raw, e))?;
        if url.scheme() != "http" {
            return Err("CLI auth server currently supports only http callback URLs".to_string());
        }

        let host = url
            .host_str()
            .ok_or_else(|| "callback URL is missing a host".to_string())?;
        let port = url.port_or_known_default().ok_or_else(|| {
            "callback URL must include an explicit port (e.g. http://localhost:5174)".to_string()
        })?;

        if url.path() != "" && url.path() != "/" && url.path() != "/auth/callback" {
            warn!(
                "Ignoring path '{}' in callback URL override; using /auth/callback",
                url.path()
            );
        }

        let base = url[..Position::BeforePath]
            .trim_end_matches('/')
            .to_string();
        let bind_ip = match host {
            "localhost" => IpAddr::V4(Ipv4Addr::LOCALHOST),
            _ => host
                .parse()
                .map_err(|_| format!("callback host '{}' must be an IP or 'localhost'", host))?,
        };

        Ok(Self {
            bind_addr: SocketAddr::new(bind_ip, port),
            callback_base_url: base,
        })
    }
}

/// Simple authentication server for OAuth flows that runs locally and shuts down after completion.
pub struct CliAuthServer {
    bind_addr: SocketAddr,
    handler_state: OAuthHandlerState,
    shutdown_signal: Arc<AtomicBool>,
}

impl CliAuthServer {
    pub fn new(
        bind_addr: SocketAddr,
        provider_registry: Arc<dyn ProviderRegistry>,
        auth_handler: Arc<OAuthHandler>,
    ) -> Self {
        let config = CallbackConfig::from_bind_addr(bind_addr);
        Self::with_callback_config(config, provider_registry, auth_handler)
    }

    pub fn with_callback_config(
        config: CallbackConfig,
        provider_registry: Arc<dyn ProviderRegistry>,
        auth_handler: Arc<OAuthHandler>,
    ) -> Self {
        let CallbackConfig {
            bind_addr,
            callback_base_url,
        } = config;
        let shutdown_signal = Arc::new(AtomicBool::new(false));
        let handler_state =
            OAuthHandlerState::new(provider_registry, auth_handler, callback_base_url)
                .with_shutdown_signal(shutdown_signal.clone());

        Self {
            bind_addr,
            handler_state,
            shutdown_signal,
        }
    }

    pub async fn start_and_wait_for_auth(&self, timeout_secs: u64) -> Result<bool, AuthError> {
        info!("Starting CLI auth server on {}", self.bind_addr);

        let server_future = self.start_server();
        let timeout_future = self.wait_for_completion(timeout_secs);
        let shutdown_signal = self.shutdown_signal.clone();

        tokio::select! {
            server_result = server_future => server_result.map(|_| true),
            completed = timeout_future => {
                if completed {
                    Ok(true)
                } else {
                    warn!("Authentication timed out after {} seconds", timeout_secs);
                    shutdown_signal.store(true, Ordering::Relaxed);
                    Ok(false)
                }
            }
        }
    }

    async fn start_server(&self) -> Result<(), AuthError> {
        let handler_state = self.handler_state.clone();
        let shutdown_signal = self.shutdown_signal.clone();

        let server = HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(handler_state.clone()))
                .wrap(
                    Cors::default()
                        .allow_any_origin()
                        .allow_any_method()
                        .allow_any_header(),
                )
                .route("/health", web::get().to(health_check))
                .route(
                    "/auth/{provider}/authorize",
                    web::get().to(start_oauth_flow),
                )
                .route("/auth/callback", web::get().to(handle_oauth_callback))
        })
        .bind(&self.bind_addr)
        .map_err(|e| {
            AuthError::ServerError(format!("Failed to bind to {}: {}", self.bind_addr, e))
        })?
        .shutdown_timeout(5)
        .run();

        let shutdown_monitor = {
            let shutdown_signal = shutdown_signal.clone();
            let server_handle = server.handle();
            tokio::spawn(async move {
                while !shutdown_signal.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                info!("Shutdown signal received, stopping auth server");
                server_handle.stop(true).await;
            })
        };

        let result = server
            .await
            .map_err(|e| AuthError::ServerError(format!("Server error: {}", e)));
        shutdown_monitor.abort();
        result
    }

    async fn wait_for_completion(&self, timeout_secs: u64) -> bool {
        let timeout_duration = Duration::from_secs(timeout_secs);
        let start = std::time::Instant::now();

        loop {
            if self.shutdown_signal.load(Ordering::Relaxed) {
                return true;
            }
            if start.elapsed() >= timeout_duration {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn get_auth_url(
        &self,
        provider_name: &str,
        scopes: Vec<String>,
        user_id: String,
    ) -> Result<String, AuthError> {
        if !self
            .handler_state
            .provider_registry
            .is_provider_available(provider_name)
            .await
        {
            return Err(AuthError::ProviderNotFound(provider_name.to_string()));
        }

        let provider = self
            .handler_state
            .provider_registry
            .get_provider(provider_name)
            .await
            .ok_or_else(|| AuthError::ProviderNotFound(provider_name.to_string()))?;

        let auth_type = self
            .handler_state
            .provider_registry
            .get_auth_type(provider_name)
            .await
            .ok_or_else(|| {
                AuthError::InvalidConfig(format!("No config for provider: {}", provider_name))
            })?;

        let state = generate_state();
        let send_redirect_uri = matches!(
            auth_type,
            AuthType::OAuth2 {
                send_redirect_uri: true,
                ..
            }
        );
        let redirect_uri = if send_redirect_uri {
            Some(format!(
                "{}/auth/callback",
                self.handler_state.callback_base_url
            ))
        } else {
            None
        };

        let mut oauth2_state = OAuth2State::new_with_state(
            state.clone(),
            provider_name.to_string(),
            redirect_uri.clone(),
            user_id,
            scopes.clone(),
        );

        let mut pkce_challenge = None;
        if self
            .handler_state
            .provider_registry
            .requires_pkce(provider_name)
            .await
        {
            let (verifier, challenge) = generate_pkce_pair();
            oauth2_state
                .metadata
                .insert(PKCE_CODE_VERIFIER_KEY.to_string(), Value::String(verifier));
            pkce_challenge = Some(challenge);
        }

        self.handler_state
            .insert_pending_session(
                state.clone(),
                PendingSession {
                    provider_name: provider_name.to_string(),
                    user_id: oauth2_state.user_id.clone(),
                    scopes: scopes.clone(),
                    redirect_url: redirect_uri.clone(),
                },
            )
            .await;

        self.handler_state
            .auth_handler
            .store_oauth2_state(oauth2_state)
            .await?;

        let mut auth_url =
            provider.build_auth_url(&auth_type, &state, &scopes, redirect_uri.as_deref())?;
        if let Some(challenge) = pkce_challenge {
            auth_url = append_pkce_challenge(&auth_url, &challenge)?;
        }
        Ok(auth_url)
    }
}

fn generate_state() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    const STATE_LEN: usize = 32;

    let mut rng = rand::thread_rng();
    (0..STATE_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}
