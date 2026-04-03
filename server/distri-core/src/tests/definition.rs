use crate::agent::parse_agent_markdown_content;

#[tokio::test]
async fn parse_agent_definition() {
    let deepresearch = include_str!("../../../agents/deepresearch.md");

    let agent_definition = parse_agent_markdown_content(deepresearch).await.unwrap();
    assert_eq!(agent_definition.name, "deepresearch");
    assert_eq!(agent_definition.max_iterations, Some(40));
}
