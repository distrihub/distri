use crate::agent::parse_agent_markdown_content;

#[tokio::test]
async fn parse_agent_definition() {
    // Verify the generic parse path against distri.md (the canonical orchestrator agent).
    let content = include_str!("../../../agents/distri.md");
    let agent_definition = parse_agent_markdown_content(content).await.unwrap();
    assert_eq!(agent_definition.name, "distri");
    assert!(agent_definition.max_iterations.unwrap() >= 10);
}
