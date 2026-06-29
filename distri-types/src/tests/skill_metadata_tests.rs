use crate::stores::{
    DEFAULT_SKILL_MAX_TOKENS, SKILL_DESCRIPTION_CAP, SkillFrontmatter, format_skill_listing,
};
use std::collections::HashMap;

// agentskills.io frontmatter — the spec at https://agentskills.io/specification:
// required: name, description; optional: license, compatibility, metadata, allowed-tools.
// Distri-specific runtime hints (model, max_tokens, can_spawn_tasks, tags) live
// inside `metadata` so the file stays portable.

#[test]
fn skill_frontmatter_parse_full() {
    let yaml = r#"
        name: web-search
        description: Search the web for information
        license: Apache-2.0
        compatibility: Designed for Claude Code
        metadata:
            model: gpt-4.1
            max_tokens: "3000"
            can_spawn_tasks: "true"
            tags: search,web
        allowed-tools: "Bash(git:*) Read"
    "#;
    let fm: SkillFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(fm.name, "web-search");
    assert_eq!(fm.license.as_deref(), Some("Apache-2.0"));
    assert_eq!(
        fm.compatibility.as_deref(),
        Some("Designed for Claude Code")
    );
    assert_eq!(fm.allowed_tools.as_deref(), Some("Bash(git:*) Read"));
    assert_eq!(fm.model(), Some("gpt-4.1"));
    assert_eq!(fm.max_tokens(), Some(3000));
    assert!(fm.can_spawn_tasks());
    assert_eq!(fm.tags(), vec!["search".to_string(), "web".to_string()]);
}

#[test]
fn skill_frontmatter_parse_minimal() {
    let yaml = r#"name: minimal-skill"#;
    let fm: SkillFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(fm.name, "minimal-skill");
    assert!(fm.model().is_none());
    assert_eq!(fm.effective_max_tokens(), DEFAULT_SKILL_MAX_TOKENS);
    assert!(!fm.can_spawn_tasks());
}

#[test]
fn skill_frontmatter_model_preference() {
    let mut metadata = HashMap::new();
    metadata.insert("model".into(), "claude-sonnet-4".into());
    let fm = SkillFrontmatter {
        name: "smart".into(),
        metadata,
        ..Default::default()
    };
    assert_eq!(fm.model(), Some("claude-sonnet-4"));
}

#[test]
fn skill_frontmatter_can_spawn_tasks() {
    let mut metadata = HashMap::new();
    metadata.insert("can_spawn_tasks".into(), "true".into());
    let fm = SkillFrontmatter {
        name: "orch".into(),
        metadata,
        ..Default::default()
    };
    assert!(fm.can_spawn_tasks());
}

#[test]
fn skill_frontmatter_max_tokens() {
    let mut metadata = HashMap::new();
    metadata.insert("max_tokens".into(), "3000".into());
    let fm = SkillFrontmatter {
        name: "test".into(),
        metadata,
        ..Default::default()
    };
    assert_eq!(fm.effective_max_tokens(), 3000);
    let fm_default = SkillFrontmatter {
        name: "test".into(),
        ..Default::default()
    };
    assert_eq!(fm_default.effective_max_tokens(), DEFAULT_SKILL_MAX_TOKENS);
}

#[test]
fn skill_listing_format_one_line() {
    let mut metadata = HashMap::new();
    metadata.insert("model".into(), "gpt-4.1".into());
    metadata.insert("can_spawn_tasks".into(), "true".into());
    let fm = SkillFrontmatter {
        name: "web-search".into(),
        description: Some("Search the web".into()),
        metadata,
        ..Default::default()
    };
    assert_eq!(
        fm.as_listing_line(),
        "- web-search: Search the web (model: gpt-4.1, tasks: yes)"
    );
}

#[test]
fn skill_listing_budget_capped() {
    let skills: Vec<SkillFrontmatter> = (0..50)
        .map(|i| SkillFrontmatter {
            name: format!("skill-{}", i),
            description: Some(format!(
                "Description for skill {} that is moderately long",
                i
            )),
            ..Default::default()
        })
        .collect();
    let listing = format_skill_listing(&skills, 50);
    let lines: Vec<&str> = listing.lines().collect();
    assert!(lines.len() < 50);
}

#[test]
fn skill_description_cap_truncates() {
    let long_desc = "x".repeat(SKILL_DESCRIPTION_CAP + 100);
    let fm = SkillFrontmatter {
        name: "test".into(),
        description: Some(long_desc),
        ..Default::default()
    };
    let line = fm.as_listing_line();
    assert!(line.contains("..."));
}
