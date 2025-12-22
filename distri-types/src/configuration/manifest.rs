use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

// Import config types
use crate::agent::{BrowserHooksConfig, ModelSettings};
use crate::configuration::config::{ExternalMcpServer, ServerConfig, StoreConfig};

/// User configuration from distri.toml file
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DistriConfiguration {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents: Option<Vec<String>>,

    // Entry points for TypeScript and WASM
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoints: Option<EntryPoints>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub distri: Option<EngineConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<AuthorsConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<RegistryConfig>,

    // Configuration that was previously in Configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<Vec<ExternalMcpServer>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerConfig>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_settings: Option<ModelSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis_model_settings: Option<ModelSettings>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stores: Option<StoreConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks: Option<BrowserHooksConfig>,
    /// Optional filesystem/object storage configuration for workspace/session files
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<crate::configuration::config::ObjectStorageConfig>,
}

/// Build configuration for custom build commands
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildConfig {
    /// Build command to execute
    pub command: String,
    /// Working directory for build (optional, defaults to package root)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// Environment variables for build (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<std::collections::HashMap<String, String>>,
}

impl DistriConfiguration {
    pub fn has_entrypoints(&self) -> bool {
        self.entrypoints.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "lowercase")]
pub struct EntryPoints {
    pub path: String,
}

impl EntryPoints {
    /// Validate the entrypoint configuration
    pub fn validate(&self) -> Result<()> {
        if self.path.is_empty() {
            return Err(anyhow!("TypeScript entrypoint path cannot be empty"));
        }
        // Basic validation - could add more checks here
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    pub engine: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorsConfig {
    pub primary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockFile {
    pub packages: HashMap<String, String>,
    pub sources: HashMap<String, String>,
}

impl DistriConfiguration {
    /// Get the working directory with fallback chain: config -> DISTRI_HOME -> current_dir
    pub fn get_working_directory(&self) -> Result<std::path::PathBuf> {
        // Fallback to current directory
        std::env::current_dir().map_err(|e| anyhow!("Failed to get current directory: {}", e))
    }

    pub async fn load_from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path).await?;
        let manifest: DistriConfiguration = toml::from_str(&content)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate that the manifest has at least one of agents or entrypoints
    pub fn validate(&self) -> Result<()> {
        let has_agents = self.agents.as_ref().map_or(false, |a| !a.is_empty());
        let has_entrypoints = self.entrypoints.is_some();

        if !has_agents && !has_entrypoints {
            return Err(anyhow!(
                "Package '{}' must define either agents or entrypoints (for tools/workflows)",
                self.name
            ));
        }

        // Validate entrypoints if present
        if let Some(entrypoints) = &self.entrypoints {
            entrypoints.validate()?;
        }

        Ok(())
    }

    pub async fn save_to_path<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        fs::write(path, content).await?;
        Ok(())
    }

    pub fn new_minimal(package_name: String) -> Self {
        Self {
            name: package_name,
            version: "0.1.0".to_string(),
            description: None,
            license: Some("Apache-2.0".to_string()),
            agents: Some(vec![]),
            entrypoints: None, // Entry points for TypeScript and WASM
            distri: Some(EngineConfig {
                engine: ">=0.1.2".to_string(),
            }),
            authors: None,
            registry: None,
            mcp_servers: None,
            server: None,
            stores: None,
            model_settings: None,
            analysis_model_settings: None,
            keywords: None,
            hooks: None,
            filesystem: None,
        }
    }
    pub fn current_dir() -> Result<std::path::PathBuf> {
        let current_dir = std::env::current_dir()?;
        Ok(current_dir)
    }

    pub fn find_configuration_in_current_dir() -> Result<std::path::PathBuf> {
        let current_dir = Self::current_dir()?;
        println!("current_dir: {:?}", current_dir);
        let manifest_path = current_dir.join("distri.toml");

        if manifest_path.exists() {
            Ok(manifest_path)
        } else {
            Err(anyhow!("No distri.toml found in current directory"))
        }
    }
}

impl LockFile {
    pub async fn load_from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path).await?;
        let lock_file: LockFile = toml::from_str(&content)?;
        Ok(lock_file)
    }

    pub async fn save_to_path<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        fs::write(path, content).await?;
        Ok(())
    }

    pub fn new() -> Self {
        Self {
            packages: HashMap::new(),
            sources: HashMap::new(),
        }
    }
}

impl Default for LockFile {
    fn default() -> Self {
        Self::new()
    }
}
