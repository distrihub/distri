use std::sync::Arc;

use crate::agent::ExecutorContext;
use crate::tools::{execute_code_with_tools, FinalTool};
use crate::AgentOrchestratorBuilder;

#[tokio::test]
async fn test_execute_code_with_console_log() {
    tracing_subscriber::fmt::init();
    let context = Arc::new(ExecutorContext {
        orchestrator: Some(Arc::new(
            AgentOrchestratorBuilder::default().build().await.unwrap(),
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
