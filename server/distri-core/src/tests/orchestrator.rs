use std::sync::Arc;

use distri_types::configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig};
use distri_types::Message;

use crate::{
    agent::{parse_agent_markdown_content, ExecutorContext},
    AgentOrchestratorBuilder,
};

/// Creates a StoreConfig that uses a temporary in-memory SQLite database
/// so tests don't depend on the filesystem having a `.distri/` directory.
fn test_store_config() -> StoreConfig {
    let db_name = uuid::Uuid::new_v4();
    let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
    StoreConfig {
        metadata: MetadataStoreConfig {
            db_config: Some(DbConnectionConfig {
                database_url: db_url,
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[tokio::test]
async fn test_orchestrator_final_result_capture() {
    if std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!("skipping orchestrator test; OPENAI_API_KEY not set");
        return;
    }
    let agent = parse_agent_markdown_content(include_str!("./test_agent.md"))
        .await
        .unwrap();
    let name = agent.name.clone();
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );
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
