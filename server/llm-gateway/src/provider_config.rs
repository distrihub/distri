//! Provider → client configuration resolution.
//!
//! Converts a `ModelProvider` enum variant into the base_url, api_key, and
//! headers needed to build a `GatewayConfig`. Keeps all the provider-specific
//! logic in one place instead of a giant match in llm.rs.

use distri_types::ModelProvider;
use std::collections::HashMap;

/// Resolved connection config for an LLM provider.
#[derive(Debug, Clone)]
pub struct ProviderClientConfig {
    pub base_url: String,
    pub api_key_secret: &'static str,
    pub inline_api_key: Option<String>,
    pub project_id: Option<String>,
    pub extra_headers: HashMap<String, String>,
    pub query_params: Vec<(String, String)>,
    /// If true, also send api_key as `api-key` header (Azure style).
    pub send_api_key_header: bool,
}

impl From<&ModelProvider> for ProviderClientConfig {
    fn from(provider: &ModelProvider) -> Self {
        // Secret name is owned by `ModelProvider::api_key_secret()` — every
        // layer that resolves an API key flows through it. Don't hardcode
        // strings here; that's how the workspace_store / gateway / validator
        // got out of sync in the first place.
        let api_key_secret = provider.api_key_secret();
        match provider {
            ModelProvider::OpenAI {} => Self {
                base_url: ModelProvider::openai_base_url(),
                api_key_secret,
                inline_api_key: None,
                project_id: None,
                extra_headers: HashMap::new(),
                query_params: vec![],
                send_api_key_header: false,
            },
            ModelProvider::Anthropic { base_url, api_key } => Self {
                base_url: base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.anthropic.com".to_string()),
                api_key_secret,
                inline_api_key: api_key.clone(),
                project_id: None,
                extra_headers: HashMap::new(),
                query_params: vec![],
                send_api_key_header: false,
            },
            ModelProvider::AzureOpenAI {
                base_url,
                api_key,
                deployment,
                api_version,
            } => Self {
                base_url: format!(
                    "{}/openai/deployments/{}",
                    base_url.trim_end_matches('/'),
                    deployment
                ),
                api_key_secret,
                inline_api_key: api_key.clone(),
                project_id: None,
                extra_headers: HashMap::new(),
                query_params: vec![("api-version".to_string(), api_version.clone())],
                send_api_key_header: true,
            },
            ModelProvider::Gemini { base_url, api_key } => Self {
                base_url: base_url.clone(),
                api_key_secret,
                inline_api_key: api_key.clone(),
                project_id: None,
                extra_headers: HashMap::new(),
                query_params: vec![],
                send_api_key_header: false,
            },
            ModelProvider::AzureAiFoundry { resource, api_key } => Self {
                // `resource` is an Azure resource name, not a URL — the
                // OpenAI-compatible endpoint is derived from it.
                base_url: ModelProvider::azure_ai_foundry_base_url(resource),
                api_key_secret,
                inline_api_key: api_key.clone(),
                project_id: None,
                extra_headers: HashMap::new(),
                query_params: vec![],
                send_api_key_header: true,
            },
            ModelProvider::AwsBedrock { base_url, api_key } => Self {
                base_url: base_url.clone(),
                api_key_secret,
                inline_api_key: api_key.clone(),
                project_id: None,
                extra_headers: HashMap::new(),
                query_params: vec![],
                send_api_key_header: false,
            },
            ModelProvider::GoogleVertex {
                base_url,
                api_key,
                project_id,
            } => Self {
                base_url: base_url.clone(),
                api_key_secret,
                inline_api_key: api_key.clone(),
                project_id: project_id.clone(),
                extra_headers: HashMap::new(),
                query_params: vec![],
                send_api_key_header: false,
            },
            ModelProvider::OpenAICompatible {
                base_url,
                api_key,
                project_id,
            } => Self {
                base_url: base_url.clone(),
                api_key_secret,
                inline_api_key: api_key.clone(),
                project_id: project_id.clone(),
                extra_headers: HashMap::new(),
                query_params: vec![],
                send_api_key_header: true,
            },
            ModelProvider::AlibabaCloud { base_url, api_key } => Self {
                base_url: base_url.clone(),
                api_key_secret,
                inline_api_key: api_key.clone(),
                project_id: None,
                extra_headers: HashMap::new(),
                query_params: vec![],
                send_api_key_header: false,
            },
            // fal.ai is image-only; LLM completion is not supported. This
            // arm exists for match exhaustiveness — image generation has its
            // own dispatch in `crate::image` that talks to fal.run directly.
            ModelProvider::FalAi { api_key } => Self {
                base_url: ModelProvider::fal_ai_base_url().to_string(),
                api_key_secret,
                inline_api_key: api_key.clone(),
                project_id: None,
                extra_headers: HashMap::new(),
                query_params: vec![],
                send_api_key_header: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_config() {
        let provider = ModelProvider::OpenAI {};
        let config = ProviderClientConfig::from(&provider);
        assert_eq!(config.base_url, "https://api.openai.com/v1");
        assert_eq!(config.api_key_secret, "OPENAI_API_KEY");
        assert!(!config.send_api_key_header);
    }

    #[test]
    fn test_azure_config() {
        let provider = ModelProvider::AzureOpenAI {
            base_url: "https://myresource.openai.azure.com".to_string(),
            api_key: None,
            deployment: "gpt-4o".to_string(),
            api_version: "2024-06-01".to_string(),
        };
        let config = ProviderClientConfig::from(&provider);
        assert_eq!(
            config.base_url,
            "https://myresource.openai.azure.com/openai/deployments/gpt-4o"
        );
        assert_eq!(config.api_key_secret, "AZURE_OPENAI_API_KEY");
        assert!(config.send_api_key_header);
        assert_eq!(
            config.query_params,
            vec![("api-version".to_string(), "2024-06-01".to_string())]
        );
    }

    #[test]
    fn test_azure_config_trailing_slash() {
        let provider = ModelProvider::AzureOpenAI {
            base_url: "https://myresource.openai.azure.com/".to_string(),
            api_key: None,
            deployment: "gpt-4o".to_string(),
            api_version: "2024-06-01".to_string(),
        };
        let config = ProviderClientConfig::from(&provider);
        assert_eq!(
            config.base_url,
            "https://myresource.openai.azure.com/openai/deployments/gpt-4o"
        );
    }

    #[test]
    fn test_anthropic_config() {
        let provider = ModelProvider::Anthropic {
            base_url: None,
            api_key: None,
        };
        let config = ProviderClientConfig::from(&provider);
        assert_eq!(config.base_url, "https://api.anthropic.com");
        assert_eq!(config.api_key_secret, "ANTHROPIC_API_KEY");
    }

    #[test]
    fn test_anthropic_with_inline_key() {
        let provider = ModelProvider::Anthropic {
            base_url: Some("https://custom.anthropic.com".to_string()),
            api_key: Some("sk-test".to_string()),
        };
        let config = ProviderClientConfig::from(&provider);
        assert_eq!(config.base_url, "https://custom.anthropic.com");
        assert_eq!(config.inline_api_key, Some("sk-test".to_string()));
    }

    #[test]
    fn test_gemini_config() {
        let provider = ModelProvider::Gemini {
            base_url: ModelProvider::gemini_base_url(),
            api_key: None,
        };
        let config = ProviderClientConfig::from(&provider);
        assert!(config.base_url.contains("googleapis.com"));
        assert_eq!(config.api_key_secret, "GEMINI_API_KEY");
    }

    #[test]
    fn test_azure_ai_foundry_config() {
        let provider = ModelProvider::AzureAiFoundry {
            resource: "myproject".to_string(),
            api_key: None,
        };
        let config = ProviderClientConfig::from(&provider);
        assert_eq!(
            config.base_url,
            "https://myproject.openai.azure.com/openai/v1"
        );
        assert_eq!(config.api_key_secret, "AZURE_AI_FOUNDRY_API_KEY");
        assert!(config.send_api_key_header);
    }

    #[test]
    fn test_azure_ai_foundry_config_trims_resource() {
        // Stray whitespace/slashes in the stored resource still resolve.
        let provider = ModelProvider::AzureAiFoundry {
            resource: " myproject/ ".to_string(),
            api_key: None,
        };
        let config = ProviderClientConfig::from(&provider);
        assert_eq!(
            config.base_url,
            "https://myproject.openai.azure.com/openai/v1"
        );
    }

    #[test]
    fn test_aws_bedrock_config() {
        let provider = ModelProvider::AwsBedrock {
            base_url: "https://bedrock-runtime.us-east-1.amazonaws.com/v1".to_string(),
            api_key: None,
        };
        let config = ProviderClientConfig::from(&provider);
        assert_eq!(config.api_key_secret, "AWS_ACCESS_KEY_ID");
    }

    #[test]
    fn test_google_vertex_config() {
        let provider = ModelProvider::GoogleVertex {
            base_url: "https://us-central1-aiplatform.googleapis.com/v1".to_string(),
            api_key: None,
            project_id: Some("my-project".to_string()),
        };
        let config = ProviderClientConfig::from(&provider);
        assert_eq!(config.api_key_secret, "GOOGLE_VERTEX_API_KEY");
        assert_eq!(config.project_id, Some("my-project".to_string()));
    }

    #[test]
    fn test_openai_compat_config() {
        let provider = ModelProvider::OpenAICompatible {
            base_url: "https://api.custom.com/v1".to_string(),
            api_key: Some("key123".to_string()),
            project_id: None,
        };
        let config = ProviderClientConfig::from(&provider);
        assert_eq!(config.base_url, "https://api.custom.com/v1");
        assert_eq!(config.inline_api_key, Some("key123".to_string()));
        assert!(config.send_api_key_header);
    }

    #[test]
    fn test_from_provider_model_str_roundtrip() {
        let cases = vec![
            ("openai", "OPENAI_API_KEY"),
            ("anthropic", "ANTHROPIC_API_KEY"),
            ("gemini", "GEMINI_API_KEY"),
            ("azure_ai_foundry", "AZURE_AI_FOUNDRY_API_KEY"),
            ("aws_bedrock", "AWS_ACCESS_KEY_ID"),
            ("google_vertex", "GOOGLE_VERTEX_API_KEY"),
            ("alibaba_cloud", "DASHSCOPE_API_KEY"),
        ];
        for (provider_str, expected_secret) in cases {
            let ms = distri_types::ModelSettings::from_provider_model_str(&format!(
                "{}/test-model",
                provider_str
            ))
            .unwrap()
            .unwrap();
            let config = ProviderClientConfig::from(&ms.inner.provider);
            assert_eq!(
                config.api_key_secret, expected_secret,
                "provider {} should use secret {}",
                provider_str, expected_secret
            );
        }
    }
}
