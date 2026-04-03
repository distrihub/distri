use crate::stores::{
    DEFAULT_SKILL_MAX_TOKENS, SKILL_DESCRIPTION_CAP, SkillFrontmatter, format_skill_listing,
};

#[test]
fn skill_frontmatter_parse_full() {
    let yaml = r#"
        name: web_search
        description: Search the web for information
        tags: [search, web]
        model: gpt-4.1
        max_tokens: 3000
        can_spawn_tasks: true
        paths: ["src/**/*.rs"]
        is_public: true
    "#;
    let fm: SkillFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(fm.name, "web_search");
    assert_eq!(fm.model.as_deref(), Some("gpt-4.1"));
    assert_eq!(fm.max_tokens, Some(3000));
    assert!(fm.can_spawn_tasks);
    assert_eq!(fm.paths, vec!["src/**/*.rs"]);
}

#[test]
fn skill_frontmatter_parse_minimal() {
    let yaml = r#"name: minimal_skill"#;
    let fm: SkillFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(fm.name, "minimal_skill");
    assert!(fm.model.is_none());
    assert_eq!(fm.effective_max_tokens(), DEFAULT_SKILL_MAX_TOKENS);
    assert!(!fm.can_spawn_tasks);
}

#[test]
fn skill_frontmatter_model_preference() {
    let yaml = "name: smart\nmodel: claude-sonnet-4";
    let fm: SkillFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(fm.model.as_deref(), Some("claude-sonnet-4"));
}

#[test]
fn skill_frontmatter_can_spawn_tasks() {
    let yaml = "name: orch\ncan_spawn_tasks: true";
    let fm: SkillFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert!(fm.can_spawn_tasks);
}

#[test]
fn skill_frontmatter_paths_relevance() {
    let yaml = "name: rust\npaths: [\"src/**/*.rs\", \"Cargo.toml\"]";
    let fm: SkillFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(fm.paths.len(), 2);
}

#[test]
fn skill_frontmatter_max_tokens() {
    let fm = SkillFrontmatter {
        name: "test".into(),
        max_tokens: Some(3000),
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
    let fm = SkillFrontmatter {
        name: "web_search".into(),
        description: Some("Search the web".into()),
        model: Some("gpt-4.1".into()),
        can_spawn_tasks: true,
        ..Default::default()
    };
    assert_eq!(
        fm.as_listing_line(),
        "- web_search: Search the web (model: gpt-4.1, tasks: yes)"
    );
}

#[test]
fn skill_listing_budget_capped() {
    let skills: Vec<SkillFrontmatter> = (0..50)
        .map(|i| SkillFrontmatter {
            name: format!("skill_{}", i),
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
