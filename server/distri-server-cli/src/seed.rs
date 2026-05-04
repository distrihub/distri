//! Embed and upsert default agents + skills on OSS server startup.
//! Idempotent: agent `register` and skill `upsert_by_name` are upserts.

use anyhow::{Context, Result};
use distri_core::AgentOrchestrator;
use distri_types::configuration::AgentConfig;
use distri_types::stores::{NewSkill, SkillFrontmatter, SkillStore};

/// Bundled agent markdown from `distri/server/agents/*.md` (compile-time embedded).
const BUNDLED_AGENTS: &[(&str, &str)] = &[
    ("distri", include_str!("../../agents/distri.md")),
    ("distri_runner", include_str!("../../agents/distri_runner.md")),
    (
        "distri_browser_runner",
        include_str!("../../agents/distri_browser_runner.md"),
    ),
    ("_adhoc_base", include_str!("../../agents/_adhoc_base.md")),
    ("plan", include_str!("../../agents/plan.md")),
    ("explore", include_str!("../../agents/explore.md")),
    ("coder", include_str!("../../agents/coder.md")),
];

/// Bundled skills under `distri/server/agents/skills/`.
const BUNDLED_SKILLS: &[(&str, &str)] = &[
    ("distri_platform", include_str!("../../agents/skills/distri_platform.md")),
    ("distri-debug", include_str!("../../agents/skills/distri-debug.md")),
    ("designer", include_str!("../../agents/skills/designer.md")),
    ("code_execution", include_str!("../../agents/skills/code_execution.md")),
];

pub async fn seed_bundled_defaults(orchestrator: &AgentOrchestrator) -> Result<()> {
    seed_bundled_agents(orchestrator).await;
    seed_bundled_skills(orchestrator).await;
    Ok(())
}

async fn seed_bundled_agents(orchestrator: &AgentOrchestrator) {
    let store = &orchestrator.stores.agent_store;
    for (label, md) in BUNDLED_AGENTS {
        match distri_types::parse_agent_markdown_content(md).await {
            Ok(def) => {
                let name = def.name.clone();
                let config = AgentConfig::StandardAgent(def);
                match store.register(config).await {
                    Ok(()) => tracing::info!("Seeded bundled agent: {name}"),
                    Err(e) => tracing::warn!("Failed to register bundled agent '{name}': {e}"),
                }
            }
            Err(e) => tracing::warn!("Failed to parse bundled agent {label}.md: {e:?}"),
        }
    }
}

async fn seed_bundled_skills(orchestrator: &AgentOrchestrator) {
    let Some(store) = orchestrator.stores.skill_store.as_ref() else {
        tracing::debug!("skill_store not configured; skipping bundled skills seed");
        return;
    };

    for (label, md) in BUNDLED_SKILLS {
        match parse_bundled_skill(md, label) {
            Ok(new_skill) => {
                let name = new_skill.name.clone();
                match store.upsert_by_name(new_skill).await {
                    Ok(_) => tracing::info!("Seeded bundled skill: {name}"),
                    Err(e) => tracing::warn!("Failed to upsert bundled skill '{name}': {e}"),
                }
            }
            Err(e) => tracing::warn!("Failed to parse bundled skill {label}.md: {e:?}"),
        }
    }
}

fn parse_bundled_skill(raw: &str, label: &str) -> Result<NewSkill> {
    let trimmed = raw.trim_start();
    let rest = trimmed
        .strip_prefix("---")
        .with_context(|| format!("{label}: skill markdown must start with --- frontmatter"))?;
    let end = rest
        .find("\n---")
        .with_context(|| format!("{label}: skill markdown missing closing ---"))?;
    let fm_str = &rest[..end];
    let body = rest[end + 4..].trim_start_matches('\n').to_string();
    let fm: SkillFrontmatter = serde_yaml::from_str(fm_str)
        .with_context(|| format!("{label}: invalid YAML frontmatter"))?;

    Ok(NewSkill {
        name: fm.name.clone(),
        description: fm.description.clone(),
        content: body,
        tags: vec![],
        model: fm.model().map(|s| s.to_string()),
        context: Default::default(),
    })
}
