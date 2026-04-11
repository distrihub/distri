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

/// Built-in agent definitions embedded at compile time.
/// These are always available and registered explicitly by the orchestrator.
pub fn system_agent_definitions() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "_system/plan",
            include_str!("../../../agents/_system/plan.md"),
        ),
        (
            "_system/coder",
            include_str!("../../../agents/_system/coder.md"),
        ),
        (
            "_system/coder_lite",
            include_str!("../../../agents/_system/coder_lite.md"),
        ),
        (
            "_system/explore",
            include_str!("../../../agents/_system/explore.md"),
        ),
    ]
}

/// Parse and return all built-in agent definitions.
pub async fn load_system_agents() -> Result<Vec<distri_types::StandardDefinition>, AgentError> {
    let mut agents = Vec::new();
    for (name, content) in system_agent_definitions() {
        let definition = parse_agent_markdown_content(content).await.map_err(|e| {
            AgentError::InvalidConfiguration(format!(
                "Failed to parse built-in agent '{}': {}",
                name, e
            ))
        })?;
        agents.push(definition);
    }
    Ok(agents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_load_system_agents() {
        let agents = load_system_agents().await.unwrap();
        let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"_system/plan"), "should have plan");
        assert!(names.contains(&"_system/coder"), "should have coder");
        assert!(
            names.contains(&"_system/coder_lite"),
            "should have coder_lite"
        );
        assert!(names.contains(&"_system/explore"), "should have explore");
        assert_eq!(agents.len(), 4);
    }

    #[tokio::test]
    async fn test_load_agents_from_dir_skips_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join("_system");
        std::fs::create_dir(&subdir).unwrap();

        std::fs::write(
            dir.path().join("root_agent.md"),
            "---\nname = \"root_agent\"\ndescription = \"Root agent\"\n---\nHello",
        )
        .unwrap();

        // Agent in subdir should NOT be found
        std::fs::write(
            subdir.join("plan.md"),
            "---\nname = \"_system/plan\"\ndescription = \"Plan agent\"\n---\nPlan",
        )
        .unwrap();

        let agents = load_agents_from_dir(dir.path()).await.unwrap();
        let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"root_agent"), "should find root agent");
        assert!(
            !names.contains(&"_system/plan"),
            "should NOT find subdir agents"
        );
    }
}
