use anyhow::{anyhow, Result};
use std::path::PathBuf;
use tokio::fs;

/// Standardized path utilities for DAP operations
pub struct PluginPaths;

impl PluginPaths {
    /// Get the user's home directory
    pub fn get_home_dir() -> Result<PathBuf> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map_err(|_| anyhow!("Could not find home directory"))?;

        Ok(PathBuf::from(home))
    }

    /// Get the global DAP directory (~/.distri)
    pub async fn get_global_dap_dir() -> Result<PathBuf> {
        let global_dir = Self::get_home_dir()?.join(".distri");
        fs::create_dir_all(&global_dir).await?;
        Ok(global_dir)
    }

    /// Get the global plugins directory (~/.distri/plugins)
    pub async fn get_global_plugins_dir() -> Result<PathBuf> {
        let global_dir = Self::get_global_dap_dir().await?.join("plugins");
        fs::create_dir_all(&global_dir).await?;
        Ok(global_dir)
    }

    /// Get the local project plugins directory (.distri/plugins)
    pub fn get_local_plugins_dir() -> PathBuf {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".distri")
            .join("plugins")
    }

    /// Get the local DAP directory (.distri)
    pub fn get_local_distri_dir() -> PathBuf {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".distri")
    }

    /// Get the local workflows directory (.distri/workflows)
    pub fn get_local_workflows_dir() -> PathBuf {
        Self::get_local_distri_dir().join("workflows")
    }

    /// Get the path to a specific package in the global plugins directory
    pub async fn get_global_package_path(package_name: &str) -> Result<PathBuf> {
        Ok(Self::get_global_plugins_dir().await?.join(package_name))
    }

    /// Get the path to a specific package in the local plugins directory  
    pub fn get_local_plugin_path(package_name: &str) -> PathBuf {
        Self::get_local_plugins_dir().join(package_name)
    }

    /// Get the path to a package manifest (distri.toml) in the global directory
    pub async fn get_global_plugin_manifest_path(package_name: &str) -> Result<PathBuf> {
        Ok(Self::get_global_package_path(package_name)
            .await?
            .join("distri.toml"))
    }

    /// Get the path to a package manifest (distri.toml) in the local directory
    pub fn get_local_plugin_manifest_path(package_name: &str) -> PathBuf {
        Self::get_local_plugin_path(package_name).join("distri.toml")
    }

    /// Check if a package exists globally
    pub async fn global_plugin_exists(package_name: &str) -> bool {
        match Self::get_global_plugin_manifest_path(package_name).await {
            Ok(manifest_path) => manifest_path.exists(),
            Err(_) => false,
        }
    }

    /// Check if a package exists locally
    pub fn local_plugin_exists(package_name: &str) -> bool {
        Self::get_local_plugin_manifest_path(package_name).exists()
    }

    /// Create all necessary DAP directories
    pub async fn ensure_distri_directories() -> Result<()> {
        // Ensure global directories
        Self::get_global_plugins_dir().await?;

        // Ensure local directories
        let local_dap_dir = Self::get_local_distri_dir();
        let local_packages_dir = Self::get_local_plugins_dir();
        let local_workflows_dir = Self::get_local_workflows_dir();

        fs::create_dir_all(&local_dap_dir).await.unwrap_or_default();
        fs::create_dir_all(&local_packages_dir)
            .await
            .unwrap_or_default();
        fs::create_dir_all(&local_workflows_dir)
            .await
            .unwrap_or_default();

        Ok(())
    }

    /// Get the path for version information file for a global package
    pub async fn get_global_package_version_info_path(package_name: &str) -> Result<PathBuf> {
        Ok(Self::get_global_package_path(package_name)
            .await?
            .join(".distri-version"))
    }
}
