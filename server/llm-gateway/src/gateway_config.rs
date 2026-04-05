use std::{collections::HashMap, str::FromStr};

use async_openai::config::Config;
use reqwest::header::{HeaderMap, HeaderName, AUTHORIZATION};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

/// Project header
pub const GATEWAY_PROJECT_HEADER: &str = "X-Project-Id";

/// Lightweight context for injecting tracing headers into LLM requests.
#[derive(Clone, Debug, Default)]
pub struct GatewayContext {
    pub thread_id: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewayConfig {
    api_base: String,
    #[serde(skip)]
    api_key: SecretString,
    project_id: String,
    #[serde(skip)]
    context: Option<GatewayContext>,
    additional_headers: Option<HashMap<String, String>>,
    #[serde(skip)]
    query_params: Vec<(String, String)>,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            api_base: std::env::var("GATEWAY_API_BASE").unwrap_or_else(|_| "".to_string()),
            api_key: std::env::var("GATEWAY_API_KEY")
                .unwrap_or_else(|_| "".to_string())
                .into(),
            project_id: std::env::var("GATEWAY_PROJECT_ID").unwrap_or_else(|_| "".to_string()),
            context: None,
            additional_headers: None,
            query_params: vec![],
        }
    }
}

impl GatewayConfig {
    /// Create client with default [GATEWAY_API_BASE] url and default API key from GATEWAY_API_KEY env var
    pub fn new() -> Self {
        Default::default()
    }

    /// Non default project id
    pub fn with_project_id<S: Into<String>>(mut self, project_id: S) -> Self {
        self.project_id = project_id.into();
        self
    }

    /// To use a different API key different from default GATEWAY_API_KEY env var
    pub fn with_api_key<S: Into<String>>(mut self, api_key: S) -> Self {
        self.api_key = SecretString::from(api_key.into());
        self
    }

    /// To use a API base url different from default [GATEWAY_API_BASE]
    pub fn with_api_base<S: Into<String>>(mut self, api_base: S) -> Self {
        self.api_base = api_base.into();
        self
    }

    pub fn with_context(mut self, context: GatewayContext) -> Self {
        self.context = Some(context);
        self
    }

    pub fn with_additional_headers(mut self, additional_headers: HashMap<String, String>) -> Self {
        self.additional_headers = Some(additional_headers);
        self
    }

    pub fn with_query_param<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        self.query_params.push((key.into(), value.into()));
        self
    }
}

impl Config for GatewayConfig {
    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();

        if !self.project_id.is_empty() {
            headers.insert(
                GATEWAY_PROJECT_HEADER,
                self.project_id.as_str().parse().unwrap(),
            );
        }

        let secret = self.api_key.expose_secret();
        if !secret.is_empty() {
            headers.insert(
                AUTHORIZATION,
                format!("Bearer {}", secret).as_str().parse().unwrap(),
            );
        }

        if let Some(context) = &self.context {
            if let Some(ref thread_id) = context.thread_id {
                headers.insert("X-Thread-Id", thread_id.clone().parse().unwrap());
            }
            if let Some(ref run_id) = context.run_id {
                headers.insert("X-Run-Id", run_id.clone().parse().unwrap());
            }
        }

        if let Some(additional_headers) = &self.additional_headers {
            for (key, value) in additional_headers.iter() {
                headers.insert(HeaderName::from_str(key).unwrap(), value.parse().unwrap());
            }
        }

        headers
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_base, path)
    }

    fn api_base(&self) -> &str {
        &self.api_base
    }

    fn api_key(&self) -> &secrecy::SecretBox<str> {
        &self.api_key
    }

    fn query(&self) -> Vec<(&str, &str)> {
        self.query_params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }
}
