use std::sync::Arc;

use distri_types::Message;

use crate::{
    agent::{parse_agent_markdown_content, ExecutorContext},
    AgentOrchestratorBuilder,
};

#[tokio::test]
async fn test_orchestrator_final_result_capture() {
    let agent = parse_agent_markdown_content(include_str!("./test_agent.md"))
        .await
        .unwrap();
    let name = agent.name.clone();
    let orchestrator = Arc::new(AgentOrchestratorBuilder::default().build().await.unwrap());
    let context = Arc::new(ExecutorContext {
        orchestrator: Some(orchestrator.clone()),
        verbose: true,
        ..Default::default()
    });
    orchestrator.register_agent_definition(agent).await.unwrap();
    let result = orchestrator
        .execute(
            &name.as_str(),
            Message::user("Test final result".to_string(), None),
            context,
            None,
        )
        .await;
    assert!(result.is_ok());
    let content = result.unwrap().content;
    println!("Content: {:?}", content);
    assert!(content.is_some());
}
