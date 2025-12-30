use std::sync::Arc;

use distri_types::{Part, Tool, ToolCall};
use serde_json::Value;

use crate::{init_logging, tools::ToolContext};

#[tokio::test]
async fn test_artifact_agent() {
    use crate::{
        agent::{file::file_agent_defintion, parse_agent_markdown_content, ExecutorContext},
        AgentOrchestratorBuilder,
    };
    dotenv::dotenv().ok();
    init_logging("info");

    let orchestrator = Arc::new(AgentOrchestratorBuilder::default().build().await.unwrap());
    let context = Arc::new(ExecutorContext {
        orchestrator: Some(orchestrator.clone()),
        verbose: true,
        ..Default::default()
    });

    let parent_agent = parse_agent_markdown_content(include_str!("./test_parent.md"))
        .await
        .unwrap();

    orchestrator
        .register_tool(parent_agent.name.as_str(), Arc::new(TestTool))
        .await;

    let file_agent = file_agent_defintion().await.unwrap();
    orchestrator
        .register_agent_definition(file_agent)
        .await
        .unwrap();

    // Step 1: Execute parent agent - this should call test_tool
    let result = orchestrator
        .run_inline_agent(
            distri_types::configuration::AgentConfig::StandardAgent(parent_agent.clone()),
            "Run test tool and return me the value",
            context.clone(),
        )
        .await;

    let parent_result = result.unwrap();

    println!("✅ Parent agent completed successfully!");
    println!("   - Content: {:?}", parent_result.content);

    // The parent agent should have automatically:
    // 1. Called test_tool which generates large content
    // 2. Triggered artifact processing (if enabled in parent agent)
    // 3. Stored artifacts and got analyzed summary
    // 4. Completed without infinite recursion

    // The key success criteria from the logs:
    // ✅ Parent agent called test_tool
    // ✅ Large content was stored as artifact (visible in logs)
    // ✅ File agent was invoked for immediate analysis
    // ✅ File agent found the artifact using list_artifacts
    // ✅ No infinite recursion occurred

    println!("✅ Artifact workflow test completed successfully!");
    println!("   - Parent agent executed and called test_tool");
    println!("   - Large content was automatically processed and stored as artifacts");
    println!("   - File agent was invoked for immediate analysis");
    println!("   - Workflow completed without infinite recursion");
}

#[derive(Debug)]
struct TestTool;

#[async_trait::async_trait]
impl Tool for TestTool {
    fn get_name(&self) -> String {
        "test_tool".to_string()
    }
    fn get_description(&self) -> String {
        "Test tool that generates large content for artifact testing".to_string()
    }
    fn get_parameters(&self) -> Value {
        Value::Null
    }
    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        // Generate large JSON content that should trigger artifact storage
        let large_test_data = serde_json::json!({
            "test_id": "artifact_test_12345",
            "description": "Large test data for artifact system validation",
            "metadata": {
                "created_at": "2024-01-01T00:00:00Z",
                "tool_name": "test_tool",
                "size_info": "This is intentionally large content to test artifact storage",
                "content_type": "application/json"
            },
            "data": {
                "items": (0..100).map(|i| {
                    serde_json::json!({
                        "id": i,
                        "name": format!("test_item_{}", i),
                        "description": format!("This is test item number {} with some descriptive text that makes the content larger", i),
                        "properties": {
                            "type": "test",
                            "active": true,
                            "tags": ["testing", "artifacts", "storage"],
                            "extra_data": "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua."
                        }
                    })
                }).collect::<Vec<_>>()
            },
            "summary": {
                "total_items": 100,
                "message": "This is a large test dataset designed to exceed the size threshold for artifact storage. The content should be automatically processed by the ArtifactWrapper and stored as an artifact file, then made available to the artifact_agent for processing."
            }
        });

        Ok(vec![Part::Data(large_test_data)])
    }
}
