use distri_js_sandbox::JsExecutor;

#[derive(Debug, Clone, Default)]
pub struct CodeExecutor {}

#[async_trait::async_trait]
impl JsExecutor for CodeExecutor {
    async fn execute(
        &self,
        name: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, rustyscript::Error> {
        let str = format!("[EchoJsExecutor]:Executing function: {name} with args: {args:?}");
        Ok(serde_json::Value::String(str))
    }
}
