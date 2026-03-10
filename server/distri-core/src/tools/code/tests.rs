use std::sync::Arc;

use distri_types::configuration::{DbConnectionConfig, MetadataStoreConfig, StoreConfig};

use crate::agent::ExecutorContext;
use crate::tools::{execute_code_with_tools, FinalTool};
use crate::AgentOrchestratorBuilder;

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
async fn test_execute_code_with_console_log() {
    let _ = tracing_subscriber::fmt::try_init();
    let context = Arc::new(ExecutorContext {
        orchestrator: Some(Arc::new(
            AgentOrchestratorBuilder::default()
                .with_store_config(test_store_config())
                .build()
                .await
                .unwrap(),
        )),
        ..Default::default()
    });
    let context = Arc::new(context.clone_with_tools(vec![Arc::new(FinalTool)]).await);

    let code = r#"
        console.log("Hello, world!");
        console.log("Test observation");
        final({"message": "Success"});
    "#;
    let (_, observations, _) = execute_code_with_tools(code, context).await.unwrap();

    assert!(observations[0].contains("Hello, world!"));
    assert!(observations[1].contains("Test observation"));
}
