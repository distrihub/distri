//! MCP Apps (SEP-1865, extension `io.modelcontextprotocol/ui`): the bits of
//! the spec the agent runtime needs.
//!
//! A server says a tool has a UI by putting `_meta.ui.resourceUri =
//! "ui://…"` on the tool **definition** (the older flat key
//! `_meta["ui/resourceUri"]` is still read). `_meta.ui.visibility` says who
//! may call the tool: `"model"` (the agent) and/or `"app"` (the UI). A tool
//! without `"model"` is never offered to the LLM; it exists for its app.
//!
//! The runtime does not render anything: it turns a call of such a tool into a
//! `Part::ResourceLink` whose `meta.distri` names the server and tool, so a
//! host (distrijs) can `resources/read` the HTML and drive the app with the
//! call's input and result.

use serde_json::{json, Value};

/// The extension id in client and server capabilities.
pub const EXTENSION_ID: &str = "io.modelcontextprotocol/ui";
/// The MIME type of an MCP App's HTML resource.
pub const APP_MIME_TYPE: &str = "text/html;profile=mcp-app";

/// What distri declares under `capabilities.extensions[EXTENSION_ID]`.
pub fn client_extension() -> Value {
    json!({ "mimeTypes": [APP_MIME_TYPE] })
}

/// `_meta.ui.resourceUri` (or the deprecated `_meta["ui/resourceUri"]`).
pub fn ui_resource_uri(meta: Option<&Value>) -> Option<&str> {
    let meta = meta?;
    meta.get("ui")
        .and_then(|ui| ui.get("resourceUri"))
        .and_then(|v| v.as_str())
        .or_else(|| meta.get("ui/resourceUri").and_then(|v| v.as_str()))
        .filter(|uri| uri.starts_with("ui://"))
}

/// Who may call a tool, from `_meta.ui.visibility`. Absent means both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility {
    pub model: bool,
    pub app: bool,
}

pub fn visibility(meta: Option<&Value>) -> Visibility {
    let list = meta
        .and_then(|m| m.get("ui"))
        .and_then(|ui| ui.get("visibility"))
        .and_then(|v| v.as_array());
    match list {
        None => Visibility {
            model: true,
            app: true,
        },
        Some(items) => {
            let has = |w: &str| items.iter().any(|v| v.as_str() == Some(w));
            Visibility {
                model: has("model"),
                app: has("app"),
            }
        }
    }
}

/// The `meta.distri` block a host reads to talk back to the app's server:
/// which connection served it, which tool was called, and the parts of the
/// `CallToolResult` an app is sent in `ui/notifications/tool-result`
/// (text content and `structuredContent`; images and blobs are left out to
/// keep the stored part small).
pub fn host_meta(server: &str, tool: &str, raw_result: &Value) -> Value {
    let text_content: Vec<Value> = raw_result
        .get("content")
        .and_then(|c| c.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("text"))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let mut result = json!({ "content": text_content });
    if let Some(sc) = raw_result.get("structuredContent") {
        result["structuredContent"] = sc.clone();
    }
    if let Some(e) = raw_result.get("isError") {
        result["isError"] = e.clone();
    }
    json!({ "server": server, "tool": tool, "result": result })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nested_and_legacy_resource_uri() {
        let nested = json!({"ui": {"resourceUri": "ui://browser/view"}});
        assert_eq!(ui_resource_uri(Some(&nested)), Some("ui://browser/view"));
        let legacy = json!({"ui/resourceUri": "ui://old/view"});
        assert_eq!(ui_resource_uri(Some(&legacy)), Some("ui://old/view"));
        let not_ui = json!({"ui": {"resourceUri": "https://x"}});
        assert_eq!(ui_resource_uri(Some(&not_ui)), None);
        assert_eq!(ui_resource_uri(None), None);
    }

    #[test]
    fn visibility_defaults_to_both() {
        assert_eq!(
            visibility(None),
            Visibility {
                model: true,
                app: true
            }
        );
        let app_only = json!({"ui": {"visibility": ["app"]}});
        assert_eq!(
            visibility(Some(&app_only)),
            Visibility {
                model: false,
                app: true
            }
        );
    }

    #[test]
    fn host_meta_keeps_text_and_structured_content() {
        let raw = json!({
            "content": [
                {"type": "text", "text": "opened"},
                {"type": "image", "data": "AAAA", "mimeType": "image/png"}
            ],
            "structuredContent": {"session_id": "s1"}
        });
        let m = host_meta("browser", "open", &raw);
        assert_eq!(m["server"], "browser");
        assert_eq!(m["result"]["content"].as_array().unwrap().len(), 1);
        assert_eq!(m["result"]["structuredContent"]["session_id"], "s1");
    }
}
