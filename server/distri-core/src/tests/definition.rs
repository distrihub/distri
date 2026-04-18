use crate::agent::parse_agent_markdown_content;

#[tokio::test]
async fn parse_agent_definition() {
    // deepresearch.md was removed; verify the generic parse path against coder.md.
    let content = include_str!("../../../agents/coder.md");
    let agent_definition = parse_agent_markdown_content(content).await.unwrap();
    assert_eq!(agent_definition.name, "coder");
    assert!(agent_definition.max_iterations.unwrap() >= 10);
}
