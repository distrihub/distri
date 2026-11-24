//! Services a workflow run (and an agent) can lean on, provided by the host.
//!
//! The engine in `distri-core` does not know where a message goes, where code runs or where
//! state lives; the host (the OSS server, the CLI, distri-cloud) gives the orchestrator one
//! of each:
//!
//! - [`Notifier`]: deliver a message to a person ("the owner") or to a channel. Backs the
//!   workflow `notify` step and the `notify` tool.
//! - [`CodeRunner`]: run a `@distri/tool` TypeScript module (stdin JSON in, stdout JSON out).
//!   Backs the workflow `code` step. The OSS server runs Deno on the host; the cloud runs it in
//!   the run's sandbox.
//! - [`WorkflowStateStore`]: a small JSON document kept per workflow between runs, read as
//!   `{state.x}` and written by the `set_state` step (e.g. "was the service up last time").

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

/// Who a notification is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum NotifyTarget {
    /// The person who owns the workflow or agent: the host resolves it (in the cloud, the
    /// owner's paired chats, falling back to the workspace's private chats).
    #[default]
    Owner,
    /// One channel, by id.
    Channel { channel_id: String },
}

impl NotifyTarget {
    /// Parse the short form used in workflow JSON and tool input: `"owner"` or a channel id
    /// (`"channel:<id>"` or a bare id).
    pub fn parse(s: &str) -> Self {
        let s = s.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("owner") || s.eq_ignore_ascii_case("me") {
            return NotifyTarget::Owner;
        }
        let id = s.strip_prefix("channel:").unwrap_or(s);
        NotifyTarget::Channel {
            channel_id: id.to_string(),
        }
    }
}

/// Where the notification comes from.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotifyOrigin {
    pub workspace_id: Option<String>,
    pub user_id: Option<String>,
    /// The agent or workflow sending it.
    pub agent_id: String,
    pub thread_id: Option<String>,
    /// The channel the run came from, if any (a bot chat).
    pub channel_id: Option<String>,
}

/// One message to deliver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyRequest {
    pub to: NotifyTarget,
    pub text: String,
    /// Optional buttons, rows top to bottom.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buttons: Vec<Vec<crate::channel_commands::ChannelButton>>,
}

/// Where a notification went.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotifyOutcome {
    /// Channels it was delivered to (ids).
    pub delivered: Vec<String>,
    /// Why some or all deliveries failed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

impl NotifyOutcome {
    pub fn ok(&self) -> bool {
        !self.delivered.is_empty()
    }
}

#[async_trait]
pub trait Notifier: Send + Sync + 'static {
    async fn notify(
        &self,
        origin: &NotifyOrigin,
        request: &NotifyRequest,
    ) -> Result<NotifyOutcome, String>;
}

/// What a code step may touch. Only network hosts: code steps never read or write the host's
/// files, and never see its environment beyond what the step passes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodePermissions {
    /// Hosts (`host` or `host:port`) for `--allow-net`. `"*"` means any host.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub net: BTreeSet<String>,
}

/// One run of a `@distri/tool` module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeRunRequest {
    /// TypeScript source of the module (its default export or `tool.run` is called).
    pub source: String,
    /// A name for logs and errors (the module path, or the step id).
    pub name: String,
    /// The JSON document written to stdin. Secrets a module needs (connection tokens) come in
    /// here through the step's input mapping (`"token": "{env.GITHUB_TOKEN}"`), never through
    /// the process environment.
    pub input: serde_json::Value,
    pub permissions: CodePermissions,
    pub timeout_secs: u64,
    pub workspace_id: Option<String>,
    /// The workflow or agent running it (the sandbox key in the cloud).
    pub owner_id: String,
    /// The script SDK (`@distri/runtime`): where the script reaches its run, with a run token.
    /// `None` when the host offers no SDK. The runner adds the URL's host to `--allow-net`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk: Option<crate::script_sdk::CodeSdkAccess>,
}

/// What a code run printed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodeRunOutput {
    /// The stdout document (`{"output": text}` when it was not JSON).
    pub value: serde_json::Value,
    /// stderr lines (logs), capped by the runner.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub logs: Vec<String>,
    pub duration_ms: u64,
}

#[async_trait]
pub trait CodeRunner: Send + Sync + 'static {
    /// Run the module. `Err` is a failed run (non-zero exit, timeout, missing runtime); the
    /// message is what the workflow records.
    async fn run_code(&self, request: CodeRunRequest) -> Result<CodeRunOutput, String>;
}

/// Which workflow a state document belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowStateKey {
    /// `""` when the host has no workspaces (OSS, CLI).
    pub workspace_id: String,
    pub workflow_id: String,
}

impl WorkflowStateKey {
    pub fn new(workspace_id: Option<&str>, workflow_id: &str) -> Self {
        Self {
            workspace_id: workspace_id.unwrap_or_default().to_string(),
            workflow_id: workflow_id.to_string(),
        }
    }
}

#[async_trait]
pub trait WorkflowStateStore: Send + Sync + 'static {
    /// The document, `{}` when there is none.
    async fn get_state(&self, key: &WorkflowStateKey) -> anyhow::Result<serde_json::Value>;
    /// Shallow-merge `patch` (an object) into the document; a `null` value removes the key.
    /// Returns the new document.
    async fn merge_state(
        &self,
        key: &WorkflowStateKey,
        patch: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value>;
}

/// Shallow merge used by every [`WorkflowStateStore`]: `null` deletes a key.
pub fn merge_state_document(
    current: &serde_json::Value,
    patch: &serde_json::Value,
) -> serde_json::Value {
    let mut out = current.as_object().cloned().unwrap_or_default();
    if let Some(p) = patch.as_object() {
        for (k, v) in p {
            if v.is_null() {
                out.remove(k);
            } else {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    serde_json::Value::Object(out)
}

/// A process-local [`WorkflowStateStore`] (OSS server, CLI, tests).
#[derive(Default)]
pub struct InMemoryWorkflowStateStore {
    docs: tokio::sync::RwLock<HashMap<WorkflowStateKey, serde_json::Value>>,
}

#[async_trait]
impl WorkflowStateStore for InMemoryWorkflowStateStore {
    async fn get_state(&self, key: &WorkflowStateKey) -> anyhow::Result<serde_json::Value> {
        Ok(self
            .docs
            .read()
            .await
            .get(key)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})))
    }

    async fn merge_state(
        &self,
        key: &WorkflowStateKey,
        patch: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let mut docs = self.docs.write().await;
        let current = docs
            .get(key)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let next = merge_state_document(&current, &patch);
        docs.insert(key.clone(), next.clone());
        Ok(next)
    }
}

/// Parse the permissions a `@distri/tool` module declares in its frontmatter (the TOML between
/// `// ---` fences in its leading comments). Only `net` is honoured for code steps; `read`
/// is refused by hosts that run untrusted code.
pub fn parse_code_permissions(source: &str) -> Result<(CodePermissions, Option<u64>), String> {
    #[derive(Deserialize, Default)]
    struct Fm {
        #[serde(default)]
        permissions: FmPerms,
        #[serde(default)]
        tool: FmTool,
    }
    #[derive(Deserialize, Default)]
    struct FmPerms {
        #[serde(default)]
        net: BTreeSet<String>,
        #[serde(default)]
        read: BTreeSet<String>,
    }
    #[derive(Deserialize, Default)]
    struct FmTool {
        #[serde(default)]
        timeout: Option<u64>,
    }

    let mut lines = source
        .lines()
        .map(str::trim)
        .skip_while(|l| l.is_empty() || l.starts_with("#!"));
    let body = |l: &str| -> Option<String> {
        l.strip_prefix("//")
            .map(|b| b.strip_prefix(' ').unwrap_or(b).trim_end().to_string())
    };
    if lines.next().and_then(body).as_deref() != Some("---") {
        return Ok((CodePermissions::default(), None));
    }
    let mut toml_src = String::new();
    for line in lines {
        let Some(b) = body(line) else {
            return Err("frontmatter: `// ---` opened but not closed".to_string());
        };
        if b == "---" {
            let fm: Fm = toml::from_str(&toml_src).map_err(|e| format!("frontmatter: {e}"))?;
            if !fm.permissions.read.is_empty() {
                return Err(
                    "frontmatter: code steps cannot read files ([permissions] read)".to_string(),
                );
            }
            return Ok((
                CodePermissions {
                    net: fm.permissions.net,
                },
                fm.tool.timeout,
            ));
        }
        toml_src.push_str(&b);
        toml_src.push('\n');
    }
    Err("frontmatter: `// ---` opened but never closed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notify_target_parses_short_forms() {
        assert_eq!(NotifyTarget::parse("owner"), NotifyTarget::Owner);
        assert_eq!(NotifyTarget::parse(""), NotifyTarget::Owner);
        assert_eq!(
            NotifyTarget::parse("channel:abc"),
            NotifyTarget::Channel {
                channel_id: "abc".into()
            }
        );
    }

    #[test]
    fn merge_deletes_nulls() {
        let cur = serde_json::json!({"a": 1, "b": 2});
        let next = merge_state_document(&cur, &serde_json::json!({"a": null, "c": 3}));
        assert_eq!(next, serde_json::json!({"b": 2, "c": 3}));
    }

    #[test]
    fn frontmatter_net_and_timeout() {
        let src = "// ---\n// [permissions]\n// net = [\"api.acme.com\"]\n// [tool]\n// timeout = 10\n// ---\nexport default () => 1;";
        let (p, t) = parse_code_permissions(src).unwrap();
        assert!(p.net.contains("api.acme.com"));
        assert_eq!(t, Some(10));
        assert!(
            parse_code_permissions("export default 1")
                .unwrap()
                .0
                .net
                .is_empty()
        );
        let bad = "// ---\n// [permissions]\n// read = [\"/etc\"]\n// ---\n";
        assert!(parse_code_permissions(bad).is_err());
    }

    #[tokio::test]
    async fn in_memory_state_roundtrip() {
        let s = InMemoryWorkflowStateStore::default();
        let k = WorkflowStateKey::new(Some("w"), "wf");
        assert_eq!(s.get_state(&k).await.unwrap(), serde_json::json!({}));
        s.merge_state(&k, serde_json::json!({"was_ok": false}))
            .await
            .unwrap();
        assert_eq!(
            s.get_state(&k).await.unwrap(),
            serde_json::json!({"was_ok": false})
        );
    }
}
