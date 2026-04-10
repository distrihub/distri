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
    let mut dirs_to_scan = vec![dir_path.to_path_buf()];

    while let Some(current_dir) = dirs_to_scan.pop() {
        let mut entries = fs::read_dir(&current_dir).await.map_err(|e| {
            AgentError::InvalidConfiguration(format!(
                "Failed to read agents directory {}: {}",
                current_dir.display(),
                e
            ))
        })?;

        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            AgentError::InvalidConfiguration(format!(
                "Failed to iterate agents directory {}: {}",
                current_dir.display(),
                e
            ))
        })? {
            let path = entry.path();
            let file_type = entry.file_type().await.map_err(|e| {
                AgentError::InvalidConfiguration(format!(
                    "Failed to read agent entry type {}: {}",
                    path.display(),
                    e
                ))
            })?;

            if file_type.is_dir() {
                dirs_to_scan.push(path);
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
    }

    Ok(agents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_load_agents_from_dir_recurses_into_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        let builtin_dir = dir.path().join("_builtin");
        std::fs::create_dir(&builtin_dir).unwrap();

        // Agent in root
        std::fs::write(
            dir.path().join("root_agent.md"),
            "---\nname = \"root_agent\"\ndescription = \"Root agent\"\n---\nHello",
        )
        .unwrap();

        // Agent in _builtin subdir
        std::fs::write(
            builtin_dir.join("plan.md"),
            "---\nname = \"_builtin/plan\"\ndescription = \"Plan agent\"\n---\nPlan",
        )
        .unwrap();

        let agents = load_agents_from_dir(dir.path()).await.unwrap();
        let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"root_agent"), "should find root agent");
        assert!(
            names.contains(&"_builtin/plan"),
            "should find builtin agent in subdir"
        );
    }
}
