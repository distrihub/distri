use anyhow::{anyhow, Result};
use distri_types::configuration::DistriServerConfig;
use std::path::PathBuf;
use tokio::fs;

#[derive(Debug)]
pub struct Plugin {
    pub configuration: DistriServerConfig,
    pub root_path: PathBuf,
}

impl Plugin {
    pub async fn load_from_current_dir() -> Result<Self> {
        let config_path = DistriServerConfig::find_configuration_in_current_dir()?;
        let configuration = DistriServerConfig::load_from_path(&config_path).await?;
        let root_path = config_path
            .parent()
            .ok_or_else(|| anyhow!("Invalid configuration path"))?
            .to_path_buf();

        Ok(Self {
            configuration,
            root_path,
        })
    }

    pub async fn create_structure(&self) -> Result<()> {
        // Create main directories
        self.create_dir("agents").await?;

        // Create TypeScript entrypoint if entrypoints are configured
        if self.configuration.entrypoints.is_some() {
            // Get entrypoint path from configuration or use default
            if let Some(distri_types::configuration::EntryPoints { path }) =
                &self.configuration.entrypoints
            {
                let entrypoint_file = self.root_path.join(path);
                if let Some(parent) = entrypoint_file.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent).await?;
                    }
                }
            }
        }

        Ok(())
    }

    async fn create_dir(&self, relative_path: &str) -> Result<()> {
        let full_path = self.root_path.join(relative_path);
        if !full_path.exists() {
            fs::create_dir_all(full_path).await?;
        }
        Ok(())
    }

    pub fn validate_structure(&self) -> Result<Vec<String>> {
        let mut issues = Vec::new();

        if let Err(err) = self.configuration.validate() {
            issues.push(err.to_string());
        }

        // Check agents have their TOML files
        if let Some(agents) = &self.configuration.agents {
            for agent_path in agents {
                let agent_toml = self.root_path.join(agent_path);
                if !agent_toml.exists() {
                    issues.push(format!("Missing agent definition: {}", agent_path));
                }
            }
        }

        Ok(issues)
    }
}
