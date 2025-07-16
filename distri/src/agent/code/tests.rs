use std::sync::Arc;

use distri_js_sandbox::{FunctionDefinition, JsWorker, JsWorkerOptions};

use rustyscript::Error;
use serde_json::Value;

use crate::agent::code::executor::CodeExecutor;

#[tokio::test]
async fn test_echo_async() -> Result<(), Error> {
    let executor = CodeExecutor::default();
    let worker = JsWorker::new(JsWorkerOptions {
        timeout: std::time::Duration::from_secs(1),
        functions: vec![FunctionDefinition {
            name: "echo".to_string(),
            description: Some("Echo a message".to_string()),
            parameters: serde_json::json!({}),
            returns: Some("The echoed message".to_string()),
        }],
        executor: Arc::new(executor),
    })?;

    let result: Value = worker.execute("echo('Hello, world!');").unwrap();

    assert_eq!(result, "Hello, world!");

    Ok(())
}
