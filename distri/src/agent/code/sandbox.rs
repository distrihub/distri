use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

#[derive(Debug, Clone)]
pub struct JsSandbox {
    timeout: Duration,
    deno_path: String,
}

impl Default for JsSandbox {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            deno_path: "deno".to_string(),
        }
    }
}

impl JsSandbox {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            deno_path: "deno".to_string(),
        }
    }

    pub fn with_deno_path(mut self, deno_path: String) -> Self {
        self.deno_path = deno_path;
        self
    }

    pub async fn execute(&self, code: &str, functions: &[FunctionDefinition]) -> Result<Value> {
        let script = self.create_script(code, functions);
        
        let output = timeout(
            self.timeout,
            Command::new(&self.deno_path)
                .args(&["run", "--allow-all", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    let stdin = child.stdin.take().unwrap();
                    tokio::io::AsyncWriteExt::write_all(stdin, script.as_bytes())
                        .and_then(|_| child.wait_with_output())
                }),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Execution timeout"))??;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Script execution failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result = serde_json::from_str(&stdout)?;
        Ok(result)
    }

    fn create_script(&self, code: &str, functions: &[FunctionDefinition]) -> String {
        let mut script = String::new();
        
        // Add function definitions
        for func in functions {
            script.push_str(&format!(
                "function {}({}) {{\n",
                func.name,
                func.parameters.as_object()
                    .map(|obj| obj.keys().cloned().collect::<Vec<_>>().join(", "))
                    .unwrap_or_default()
            ));
            script.push_str("  // Function implementation will be injected by the executor\n");
            script.push_str("  throw new Error('Function not implemented');\n");
            script.push_str("}\n\n");
        }

        // Add the main code
        script.push_str(code);
        script.push_str("\n");

        // Add result output
        script.push_str("console.log(JSON.stringify(result));\n");

        script
    }
}

#[derive(Debug, Clone)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Value,
    pub returns: Option<String>,
}

impl FunctionDefinition {
    pub fn new(name: String) -> Self {
        Self {
            name,
            description: None,
            parameters: serde_json::json!({}),
            returns: None,
        }
    }

    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }

    pub fn with_parameters(mut self, parameters: Value) -> Self {
        self.parameters = parameters;
        self
    }

    pub fn with_returns(mut self, returns: String) -> Self {
        self.returns = Some(returns);
        self
    }
}