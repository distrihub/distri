use crate::oauth::{OAuthManager, OAuthService};
use crate::stores::AuthStore;
use crate::types::McpSession;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
pub struct OAuthInitiateRequest {
    pub service_name: String,
    pub user_id: String,
    pub redirect_uri: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OAuthInitiateResponse {
    pub authorization_url: String,
    pub state: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OAuthCallbackRequest {
    pub code: String,
    pub state: String,
    pub user_id: String,
}

pub struct OAuthHandler {
    oauth_manager: Arc<OAuthManager>,
    auth_store: Arc<dyn AuthStore>,
}

impl OAuthHandler {
    pub fn new(oauth_manager: Arc<OAuthManager>, auth_store: Arc<dyn AuthStore>) -> Self {
        Self {
            oauth_manager,
            auth_store,
        }
    }

    pub async fn initiate_oauth(
        &self,
        request: OAuthInitiateRequest,
    ) -> Result<OAuthInitiateResponse> {
        let service = self
            .oauth_manager
            .get_service(&request.service_name)
            .ok_or_else(|| anyhow::anyhow!("OAuth service not found: {}", request.service_name))?;

        let state = crate::oauth::flows::OAuthFlow::generate_state();
        let redirect_uri = request.redirect_uri.unwrap_or_else(|| {
            format!("http://localhost:8080/api/v1/oauth/callback")
        });

        // Store OAuth state
        self.auth_store
            .store_oauth_state(&state, &service.name, &request.user_id, &redirect_uri)
            .await?;

        let authorization_url = service.get_authorization_url(&state);

        Ok(OAuthInitiateResponse {
            authorization_url,
            state,
        })
    }

    pub async fn handle_callback(
        &self,
        request: OAuthCallbackRequest,
    ) -> Result<()> {
        let service = self
            .oauth_manager
            .get_service(&request.service_name)
            .ok_or_else(|| anyhow::anyhow!("OAuth service not found"))?;

        let flow = crate::oauth::flows::OAuthFlow::new();
        flow.handle_oauth_callback(
            self.auth_store.as_ref(),
            service,
            &request.user_id,
            &request.code,
            &request.state,
        )
        .await
    }

    pub async fn get_session_for_service(
        &self,
        service_name: &str,
        user_id: &str,
    ) -> Result<Option<McpSession>> {
        self.oauth_manager
            .create_session_from_tokens(self.auth_store.as_ref(), service_name, user_id)
            .await
    }

    pub async fn check_auth_required(
        &self,
        service_name: &str,
        user_id: &str,
    ) -> Result<bool> {
        self.oauth_manager
            .check_auth_required(self.auth_store.as_ref(), service_name, user_id)
            .await
    }
}