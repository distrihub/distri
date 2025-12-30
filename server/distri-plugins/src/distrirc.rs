use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;
use tokio::fs;
use tracing::info;

/// DAPRC configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaprcConfig {
    pub registry: RegistryConfig,
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub token: Option<String>,
}

impl Default for DaprcConfig {
    fn default() -> Self {
        Self {
            registry: RegistryConfig {
                url: "http://localhost:8083".to_string(),
            },
            auth: AuthConfig { token: None },
        }
    }
}

/// Initialize DAPRC configuration file with registry URL
pub async fn init_daprc(registry_url: &str, token: Option<&str>) -> Result<()> {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map_err(|_| anyhow!("Could not find home directory"))?;

    let daprc_path = PathBuf::from(home).join(".distrirc");

    let config = DaprcConfig {
        registry: RegistryConfig {
            url: registry_url.to_string(),
        },
        auth: AuthConfig {
            token: token.map(|s| s.to_string()),
        },
    };

    let daprc_content = toml::to_string_pretty(&config)
        .map_err(|e| anyhow!("Failed to serialize DAPRC config: {}", e))?;

    // Add header comment
    let daprc_content = format!(
        "# DAP Configuration File\n# Global settings for the DAP (Distri Agent Package) manager\n\n{}",
        daprc_content
    );

    fs::write(daprc_path, daprc_content).await?;
    info!("✓ Created ~/.distrirc configuration file");

    Ok(())
}

/// Parse DAPRC configuration and return config struct
pub async fn parse_daprc() -> Result<DaprcConfig> {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map_err(|_| anyhow!("Could not find home directory"))?;

    let daprc_path = PathBuf::from(home).join(".distrirc");

    if !daprc_path.exists() {
        return Ok(DaprcConfig::default());
    }

    let content = fs::read_to_string(&daprc_path).await?;

    let config = toml::from_str::<DaprcConfig>(&content)
        .map_err(|e| anyhow!("Failed to parse DAPRC config: {}", e))?;

    Ok(config)
}

/// Get registry URL from DAPRC configuration
pub async fn get_registry_url() -> String {
    match parse_daprc().await {
        Ok(config) => config.registry.url,
        Err(_) => "http://localhost:8083".to_string(),
    }
}

/// Get authentication token from DAPRC configuration
pub async fn get_token() -> Result<String> {
    let config = parse_daprc().await?;

    config
        .auth
        .token
        .ok_or_else(|| anyhow!("Not logged in. Run 'dap login' first."))
}

/// Update the token in DAPRC configuration
pub async fn update_token(token: &str) -> Result<()> {
    let config = parse_daprc().await?;
    init_daprc(&config.registry.url, Some(token)).await
}
