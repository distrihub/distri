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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_load_agents_from_dir_skips_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();

        std::fs::write(
            dir.path().join("root_agent.md"),
            "---\nname = \"root_agent\"\ndescription = \"Root agent\"\n---\nHello",
        )
        .unwrap();

        // Agent in subdir should NOT be found
        std::fs::write(
            subdir.join("nested.md"),
            "---\nname = \"nested_agent\"\ndescription = \"Nested\"\n---\nNested",
        )
        .unwrap();

        let agents = load_agents_from_dir(dir.path()).await.unwrap();
        let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"root_agent"), "should find root agent");
        assert!(
            !names.contains(&"nested_agent"),
            "should NOT find subdir agents"
        );
    }
}
