use crate::agent::parse_agent_markdown_content;

#[tokio::test]
async fn parse_agent_definition() {
    let deepagent = include_str!("../../../agents/deepagent.md");

    let agent_definition = parse_agent_markdown_content(deepagent).await.unwrap();
    assert_eq!(agent_definition.name, "deepagent");
    assert_eq!(agent_definition.max_iterations, Some(40));
}
