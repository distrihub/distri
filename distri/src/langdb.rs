use std::{collections::HashMap, str::FromStr, sync::Arc};

use async_openai::config::Config;
use reqwest::header::{HeaderMap, HeaderName, AUTHORIZATION};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;

use crate::coordinator::CoordinatorContext;
/// Using LangDB as a gateway for OpenAI
/// https://docs.langdb.ai/
pub const GATEWAY_API_BASE: &str = "https://api.us-east-1.langdb.ai/v1";
/// Project header
pub const GATEWAY_PROJECT_HEADER: &str = "X-Project-Id";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewayConfig {
    api_base: String,
    #[serde(skip)]
    api_key: SecretString,
    project_id: String,
    #[serde(skip)]
    context: Option<Arc<CoordinatorContext>>,
    additional_tags: Option<HashMap<String, String>>,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            api_base: std::env::var("GATEWAY_API_BASE")
                .unwrap_or_else(|_| GATEWAY_API_BASE.to_string()),
            api_key: std::env::var("GATEWAY_API_KEY")
                .unwrap_or_else(|_| "".to_string())
                .into(),
            project_id: std::env::var("GATEWAY_PROJECT_ID").unwrap_or_else(|_| "".to_string()),
            context: None,
            additional_tags: None,
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

    pub fn with_context(mut self, context: Arc<CoordinatorContext>) -> Self {
        self.context = Some(context);
        self
    }

    pub fn with_additional_tags(mut self, additional_tags: HashMap<String, String>) -> Self {
        self.additional_tags = Some(additional_tags);
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

        headers.insert(
            AUTHORIZATION,
            format!("Bearer {}", self.api_key.expose_secret())
                .as_str()
                .parse()
                .unwrap(),
        );
        if let Some(context) = &self.context {
            headers.insert("X-Thread-Id", context.thread_id.parse().unwrap());

            if let Ok(run_id) = context.run_id.try_lock() {
                headers.insert("X-Run-Id", run_id.parse().unwrap());
            }
        }

        if let Some(additional_tags) = &self.additional_tags {
            for (key, value) in additional_tags.iter() {
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
        vec![]
    }
}
