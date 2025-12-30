use distri_types::{parse_agent_markdown_content, AgentError};
use std::path::Path;
use tokio::fs;

pub async fn load_agents_from_dir<P: AsRef<Path>>(
    dir: P,
) -> Result<Vec<distri_types::StandardDefinition>, AgentError> {
    let dir_path = dir.as_ref();

    if !dir_path.exists() {
        return Ok(Vec::new());
    }

    let mut agents = Vec::new();
    let mut entries = fs::read_dir(dir_path).await.map_err(|e| {
        AgentError::InvalidConfiguration(format!(
            "Failed to read agents directory {}: {}",
            dir_path.display(),
            e
        ))
    })?;

    while let Some(entry) = entries.next_entry().await.map_err(|e| {
        AgentError::InvalidConfiguration(format!(
            "Failed to iterate agents directory {}: {}",
            dir_path.display(),
            e
        ))
    })? {
        let path = entry.path();
        if entry
            .file_type()
            .await
            .map_err(|e| {
                AgentError::InvalidConfiguration(format!(
                    "Failed to read agent entry type {}: {}",
                    path.display(),
                    e
                ))
            })?
            .is_dir()
        {
            continue;
        }

        let is_markdown = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("md"))
            .unwrap_or(false);

        if !is_markdown {
            continue;
        }

        let contents = fs::read_to_string(&path).await.map_err(|e| {
            AgentError::InvalidConfiguration(format!(
                "Failed to read agent markdown {}: {}",
                path.display(),
                e
            ))
        })?;

        let definition = parse_agent_markdown_content(&contents).await?;
        agents.push(definition);
    }

    Ok(agents)
}
