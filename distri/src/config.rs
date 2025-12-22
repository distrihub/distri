//! Configuration for the Distri client.
//!
//! This module re-exports `DistriClientConfig` from `distri-types` for convenience.
//! The actual implementation is in `distri-types` to allow reuse across packages.

// Re-export from distri-types
pub use distri_types::DistriClientConfig;

/// Trait for building HTTP clients from DistriClientConfig.
/// This trait is defined in distri-client because it depends on reqwest.
pub trait BuildHttpClient {
    /// Build a reqwest client with the configured settings.
    fn build_http_client(&self) -> Result<reqwest::Client, reqwest::Error>;
}

impl BuildHttpClient for DistriClientConfig {
    fn build_http_client(&self) -> Result<reqwest::Client, reqwest::Error> {
        let mut builder =
            reqwest::Client::builder().timeout(std::time::Duration::from_secs(self.timeout_secs));

        // Add default headers if API key is configured
        if let Some(ref api_key) = self.api_key {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                "x-api-key",
                reqwest::header::HeaderValue::from_str(api_key).expect("Invalid API key format"),
            );
            builder = builder.default_headers(headers);
        }

        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DistriClientConfig::default();
        assert_eq!(config.base_url, "https://api.distri.dev");
        assert!(config.api_key.is_none());
        assert!(!config.is_local());
    }

    #[test]
    fn test_local_config() {
        let config = DistriClientConfig::new("http://localhost:3033");
        assert!(config.is_local());
        assert!(!config.has_auth());
    }

    #[test]
    fn test_with_api_key() {
        let config = DistriClientConfig::default().with_api_key("test-key");
        assert!(config.has_auth());
        assert_eq!(config.api_key, Some("test-key".to_string()));
    }

    #[test]
    fn test_trailing_slash_removed() {
        let config = DistriClientConfig::new("http://localhost:3033/");
        assert_eq!(config.base_url, "http://localhost:3033");
    }
}
