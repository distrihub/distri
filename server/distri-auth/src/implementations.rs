use distri_types::auth::{AuthMetadata, AuthType, OAuth2FlowType};
use std::collections::HashMap;

/// OAuth2 authentication metadata implementation
#[derive(Debug, Clone)]
pub struct OAuth2AuthMetadata {
    pub auth_entity: String,
    pub authorization_url: String,
    pub token_url: String,
    pub refresh_url: Option<String>,
    pub scopes: Vec<String>,
    pub flow_type: OAuth2FlowType,
    pub send_redirect_uri: bool,
    pub config: HashMap<String, serde_json::Value>,
}

impl OAuth2AuthMetadata {
    pub fn new(
        auth_entity: String,
        authorization_url: String,
        token_url: String,
        scopes: Vec<String>,
    ) -> Self {
        Self {
            auth_entity,
            authorization_url,
            token_url,
            refresh_url: None,
            scopes,
            flow_type: OAuth2FlowType::AuthorizationCode,
            send_redirect_uri: true,
            config: HashMap::new(),
        }
    }

    pub fn with_refresh_url(mut self, refresh_url: String) -> Self {
        self.refresh_url = Some(refresh_url);
        self
    }

    pub fn with_flow_type(mut self, flow_type: OAuth2FlowType) -> Self {
        self.flow_type = flow_type;
        self
    }

    pub fn with_config(mut self, key: String, value: serde_json::Value) -> Self {
        self.config.insert(key, value);
        self
    }

    pub fn with_redirect_behavior(mut self, send_redirect_uri: bool) -> Self {
        self.send_redirect_uri = send_redirect_uri;
        self
    }
}

impl AuthMetadata for OAuth2AuthMetadata {
    fn get_auth_entity(&self) -> String {
        self.auth_entity.clone()
    }

    fn get_auth_type(&self) -> AuthType {
        AuthType::OAuth2 {
            flow_type: self.flow_type.clone(),
            authorization_url: self.authorization_url.clone(),
            token_url: self.token_url.clone(),
            refresh_url: self.refresh_url.clone(),
            scopes: self.scopes.clone(),
            send_redirect_uri: self.send_redirect_uri,
        }
    }

    fn get_auth_config(&self) -> HashMap<String, serde_json::Value> {
        self.config.clone()
    }
}

/// Secret-based authentication metadata implementation  
#[derive(Debug, Clone)]
pub struct SecretAuthMetadata {
    pub auth_entity: String,
    pub provider: String,
    pub fields: Vec<distri_types::auth::SecretFieldSpec>,
    pub config: HashMap<String, serde_json::Value>,
}

impl SecretAuthMetadata {
    pub fn new(
        auth_entity: String,
        provider: String,
        fields: Vec<distri_types::auth::SecretFieldSpec>,
    ) -> Self {
        Self {
            auth_entity,
            provider: provider.clone(),
            fields,
            config: HashMap::new(),
        }
    }

    pub fn with_config(mut self, key: String, value: serde_json::Value) -> Self {
        self.config.insert(key, value);
        self
    }
}

impl AuthMetadata for SecretAuthMetadata {
    fn get_auth_entity(&self) -> String {
        self.auth_entity.clone()
    }

    fn get_auth_type(&self) -> AuthType {
        AuthType::Secret {
            provider: self.provider.clone(),
            fields: self.fields.clone(),
        }
    }

    fn get_auth_config(&self) -> HashMap<String, serde_json::Value> {
        self.config.clone()
    }
}

/// No authentication metadata implementation
#[derive(Debug, Clone)]
pub struct NoAuthMetadata;

impl AuthMetadata for NoAuthMetadata {
    fn get_auth_entity(&self) -> String {
        "none".to_string()
    }

    fn get_auth_type(&self) -> AuthType {
        AuthType::None
    }

    fn requires_auth(&self) -> bool {
        false
    }
}

/// Convert A2A SecurityScheme to AuthMetadata
pub fn from_a2a_security_scheme(
    scheme: distri_types::a2a::SecurityScheme,
) -> Result<Box<dyn AuthMetadata>, String> {
    match scheme {
        distri_types::a2a::SecurityScheme::ApiKey(api_key_scheme) => {
            // Convert API key scheme to Secret auth
            Ok(Box::new(SecretAuthMetadata::new(
                api_key_scheme.name.clone(), // Use the name as entity
                api_key_scheme.name,         // Use the name as provider too
                Vec::new(),
            )))
        }
        distri_types::a2a::SecurityScheme::Oauth2(oauth2_scheme) => {
            // For now, we'll take the authorization code flow if available
            if let Some(auth_code_flow) = oauth2_scheme.flows.authorization_code {
                let token_url = auth_code_flow.token_url.clone();
                let refresh_url = auth_code_flow
                    .refresh_url
                    .unwrap_or_else(|| token_url.clone());
                Ok(Box::new(
                    OAuth2AuthMetadata::new(
                        "a2a_oauth2".to_string(), // Default entity name
                        auth_code_flow.authorization_url,
                        auth_code_flow.token_url,
                        auth_code_flow.scopes.keys().cloned().collect(),
                    )
                    .with_refresh_url(refresh_url),
                ))
            } else if let Some(client_creds_flow) = oauth2_scheme.flows.client_credentials {
                Ok(Box::new(
                    OAuth2AuthMetadata::new(
                        "a2a_oauth2_client_creds".to_string(),
                        "".to_string(), // Client credentials doesn't need auth URL
                        client_creds_flow.token_url,
                        client_creds_flow.scopes.keys().cloned().collect(),
                    )
                    .with_flow_type(OAuth2FlowType::ClientCredentials),
                ))
            } else {
                Err("No supported OAuth2 flow found".to_string())
            }
        }
        _ => Err("Unsupported A2A security scheme".to_string()),
    }
}
