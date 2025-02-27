use anyhow::{Context, Result};
use async_mcp::server::{Server, ServerBuilder};
use async_mcp::transport::Transport;
use async_mcp::types::{
    CallToolRequest, CallToolResponse, ListRequest, PromptsListResponse, Resource,
    ResourcesListResponse, ServerCapabilities, Tool, ToolResponseContent,
};
use serde_json::json;

use tracing::info;
use url::Url;

use crate::python_runner::execute_python;

pub fn build<T: Transport>(t: T) -> Result<Server<T>> {
    let mut server = Server::builder(t)
        .capabilities(ServerCapabilities {
            tools: Some(json!({})),
            ..Default::default()
        })
        .request_handler("resources/list", |_req: ListRequest| {
            Box::pin(async move { Ok(list_resources()) })
        })
        .request_handler("prompts/list", |_req: ListRequest| {
            Box::pin(async move {
                Ok(PromptsListResponse {
                    prompts: vec![],
                    next_cursor: None,
                    meta: None,
                })
            })
        });

    register_tools(&mut server)?;

    let server = server.build();
    Ok(server)
}

fn list_resources() -> ResourcesListResponse {
    let base = Url::parse("https://distr.ai/").unwrap();
    let resources = ["python"]
        .iter()
        .map(|r| Resource {
            uri: base.join(r).unwrap(),
            name: r.to_string(),
            description: None,
            mime_type: Some("text/x-python".to_string()),
        })
        .collect();
    ResourcesListResponse {
        resources,
        next_cursor: None,
        meta: None,
    }
}

fn register_tools<T: Transport>(server: &mut ServerBuilder<T>) -> Result<()> {
    let python_tool = Tool {
        name: "run_python".to_string(),
        description: Some("Execute Python code in a secure Docker container".to_string()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "Python code to execute"
                }
            },
            "required": ["code"],
            "additionalProperties": false
        }),
        output_schema: Some(json!({
            "type": "object",
            "properties": {
                "output": {"type": "string"}
            },
        })),
    };

    server.register_tool(python_tool, |req: CallToolRequest| {
        Box::pin(async move {
            let args = req.arguments.unwrap_or_default();

            let result = async {
                let code = args["code"]
                    .as_str()
                    .context("code parameter is required")?;

                let output = execute_python(code)?;

                let response = if output.exit_code == Some(0) {
                    CallToolResponse {
                        content: vec![ToolResponseContent::Text {
                            text: output.stdout,
                        }],
                        is_error: None,
                        meta: Some(json!({
                            "stderr": output.stderr,
                            "exit_code": output.exit_code
                        })),
                    }
                } else {
                    CallToolResponse {
                        content: vec![ToolResponseContent::Text {
                            text: format!("Error: {}\n{}", output.stderr, output.stdout),
                        }],
                        is_error: Some(true),
                        meta: Some(json!({
                            "exit_code": output.exit_code
                        })),
                    }
                };

                Ok(response)
            }
            .await as Result<CallToolResponse>;

            match result {
                Ok(response) => Ok(response),
                Err(e) => {
                    info!("Error handling request: {:#?}", e);
                    Ok(CallToolResponse {
                        content: vec![ToolResponseContent::Text {
                            text: format!("Internal error: {}", e),
                        }],
                        is_error: Some(true),
                        meta: None,
                    })
                }
            }
        })
    });

    Ok(())
}
