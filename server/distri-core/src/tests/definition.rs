use crate::agent::parse_agent_markdown_content;

#[tokio::test]
async fn parse_agent_definition() {
    let deepresearch = include_str!("../../../agents/deepresearch.md");

    let agent_definition = parse_agent_markdown_content(deepresearch).await.unwrap();
    assert_eq!(agent_definition.name, "deepresearch");
    assert_eq!(agent_definition.max_iterations, Some(40));
}

#[tokio::test]
async fn distri_runner_has_external_tools_and_no_shell() {
    let md = include_str!("../../../agents/distri_runner.md");
    let def = parse_agent_markdown_content(md).await.unwrap();

    assert_eq!(def.name, "distri_runner");

    // Must not have coder as a sub-agent
    let sub_agents = &def.sub_agents;
    assert!(
        !sub_agents.iter().any(|s| s == "coder"),
        "distri_runner must not have 'coder' as a sub-agent, got: {:?}",
        sub_agents
    );

    let tools = def
        .tools
        .as_ref()
        .expect("distri_runner must have tools config");

    // Must not have shell builtins
    for forbidden in &["start_shell", "execute_shell", "stop_shell"] {
        assert!(
            !tools.builtin.contains(&forbidden.to_string()),
            "distri_runner must not have '{}' as a builtin tool",
            forbidden
        );
    }

    // Must have external tools including fs and execute_command
    let external = tools
        .external
        .as_ref()
        .expect("distri_runner must have external tools");
    for required in &["fs_write_file", "fs_read_file", "execute_command"] {
        assert!(
            external.contains(&required.to_string()),
            "distri_runner must have '{}' as an external tool, got: {:?}",
            required,
            external
        );
    }
}
