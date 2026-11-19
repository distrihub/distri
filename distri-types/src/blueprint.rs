//! Blueprints: one folder that sets up an agent, its workflows and where it lives.
//!
//! ```text
//! sre-agent/
//!   blueprint.toml      # name, about, the one-sentence prompt, params, channel/embed, needs
//!   agents/*.md         # agent definitions (the usual markdown + TOML frontmatter)
//!   workflows/*.json    # workflow agents (`agent_type = "workflow_agent"`)
//!   skills/*.md         # skills
//!   tools/*.ts          # `@distri/tool` modules used by workflow `code` steps
//! ```
//!
//! [`Blueprint::load_dir`] reads the folder into the self-contained JSON form (every file
//! inlined, every `code` step's `module` resolved to its `source`); that form is what
//! `POST /v1/blueprints/apply` takes, what `distri apply` sends and what a host embeds as a
//! sample. [`Blueprint::render`] fills `{{params.x}}` placeholders before applying.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The manifest file in a blueprint folder.
pub const BLUEPRINT_FILE: &str = "blueprint.toml";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Blueprint {
    /// Slug: `sre-agent`.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// One or two sentences: what it sets up.
    #[serde(default)]
    pub about: String,
    /// The one sentence a person types to get this ("Create an SRE agent for …").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// `engineering`, `support`, `gtm`, `operations`…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Values the person gives (or the builder infers from their sentence).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, BlueprintParam>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<BlueprintFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workflows: Vec<BlueprintFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<BlueprintFile>,
    /// Code modules, kept for reference; `code` steps already carry their source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<BlueprintFile>,
    /// A chat the agent answers in (Telegram, WhatsApp).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<BlueprintChannel>,
    /// A website the agent is embedded on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed: Option<BlueprintEmbed>,
    /// Connections the agents or workflows use (OAuth).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<BlueprintConnection>,
    /// Workspace secrets the agents or workflows read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<BlueprintSecret>,
    /// A first run after applying: which workflow, with what input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<BlueprintTest>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BlueprintParam {
    /// The question to ask when it is not given: "Which URL should I check?"
    pub ask: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Required unless it has a default.
    #[serde(default = "yes")]
    pub required: bool,
}

fn yes() -> bool {
    true
}

/// One file of the blueprint, inlined.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BlueprintFile {
    /// Path inside the blueprint folder: `agents/acme_sre.md`.
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    #[default]
    Telegram,
    Whatsapp,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BlueprintChannel {
    pub kind: ChannelKind,
    /// The agent that answers in the chat.
    pub agent: String,
    /// Workflows whose `owner` notifications and schedule replies go to this chat.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workflows: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BlueprintEmbed {
    pub agent: String,
    /// Allowed origins (`https://acme.com`). May use `{{params.x}}`.
    #[serde(default)]
    pub origins: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BlueprintConnection {
    /// `github`, `linear`, `google`…
    pub provider: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    #[serde(default = "yes")]
    pub required: bool,
    /// Why it is needed, shown next to the Connect link.
    #[serde(default)]
    pub why: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BlueprintSecret {
    /// `GRAFANA_TOKEN`
    pub name: String,
    #[serde(default = "yes")]
    pub required: bool,
    #[serde(default)]
    pub why: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BlueprintTest {
    pub workflow: String,
    #[serde(default)]
    pub input: serde_json::Value,
    /// What the person should see: "a test alert arrives on Telegram".
    #[serde(default)]
    pub expect: String,
}

/// `blueprint.toml`: the manifest minus the inlined files.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    name: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    about: String,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    params: BTreeMap<String, BlueprintParam>,
    #[serde(default)]
    channel: Option<BlueprintChannel>,
    #[serde(default)]
    embed: Option<BlueprintEmbed>,
    #[serde(default)]
    connections: Vec<BlueprintConnection>,
    #[serde(default)]
    secrets: Vec<BlueprintSecret>,
    #[serde(default)]
    test: Option<BlueprintTestToml>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlueprintTestToml {
    workflow: String,
    #[serde(default)]
    input: Option<toml::Value>,
    #[serde(default)]
    expect: String,
}

/// Something wrong with a blueprint, said so a person can fix it.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum BlueprintError {
    #[error("blueprint: {0}")]
    Invalid(String),
    #[error("blueprint: missing value for '{name}': {ask}")]
    MissingParam { name: String, ask: String },
}

impl Blueprint {
    /// Parse a manifest plus a set of files (path → content) into the inlined form. Files are
    /// grouped by their top folder (`agents/`, `workflows/`, `skills/`, `tools/`); other files
    /// are ignored. `code` steps with a `module` get its `source`.
    pub fn from_parts(
        manifest_toml: &str,
        files: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, BlueprintError> {
        let m: Manifest = toml::from_str(manifest_toml)
            .map_err(|e| BlueprintError::Invalid(format!("{BLUEPRINT_FILE}: {e}")))?;
        let mut bp = Blueprint {
            name: m.name,
            title: m.title,
            about: m.about,
            prompt: m.prompt,
            category: m.category,
            params: m.params,
            channel: m.channel,
            embed: m.embed,
            connections: m.connections,
            secrets: m.secrets,
            test: m.test.map(|t| BlueprintTest {
                workflow: t.workflow,
                input: t
                    .input
                    .and_then(|v| serde_json::to_value(v).ok())
                    .unwrap_or_else(|| serde_json::json!({})),
                expect: t.expect,
            }),
            ..Default::default()
        };
        let mut files: Vec<(String, String)> = files
            .into_iter()
            .map(|(p, c)| (p.replace('\\', "/"), c))
            .collect();
        files.sort();
        for (path, content) in files {
            let file = BlueprintFile {
                path: path.clone(),
                content,
            };
            match path.split('/').next().unwrap_or("") {
                "agents" if path.ends_with(".md") => bp.agents.push(file),
                "workflows" if path.ends_with(".json") => bp.workflows.push(file),
                "skills" if path.ends_with(".md") => bp.skills.push(file),
                "tools" if path.ends_with(".ts") => bp.tools.push(file),
                _ => {}
            }
        }
        bp.inline_code_modules()?;
        bp.validate()?;
        Ok(bp)
    }

    /// Read a blueprint folder.
    pub fn load_dir(dir: &Path) -> Result<Self, BlueprintError> {
        let manifest = std::fs::read_to_string(dir.join(BLUEPRINT_FILE)).map_err(|e| {
            BlueprintError::Invalid(format!("{}: {e}", dir.join(BLUEPRINT_FILE).display()))
        })?;
        let mut files = Vec::new();
        for sub in ["agents", "workflows", "skills", "tools"] {
            let d = dir.join(sub);
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    let content = std::fs::read_to_string(&p)
                        .map_err(|e| BlueprintError::Invalid(format!("{}: {e}", p.display())))?;
                    let name = p.file_name().unwrap_or_default().to_string_lossy();
                    files.push((format!("{sub}/{name}"), content));
                }
            }
        }
        Self::from_parts(&manifest, files)
    }

    /// Workflow and agent names this blueprint creates.
    pub fn agent_names(&self) -> Vec<String> {
        self.agents
            .iter()
            .filter_map(|f| agent_name_from_markdown(&f.content))
            .collect()
    }

    pub fn workflow_names(&self) -> Vec<String> {
        self.workflows
            .iter()
            .filter_map(|f| {
                serde_json::from_str::<serde_json::Value>(&f.content)
                    .ok()
                    .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
            })
            .collect()
    }

    fn inline_code_modules(&mut self) -> Result<(), BlueprintError> {
        let tools: BTreeMap<String, String> = self
            .tools
            .iter()
            .map(|t| (t.path.clone(), t.content.clone()))
            .collect();
        for wf in &mut self.workflows {
            let mut v: serde_json::Value = serde_json::from_str(&wf.content)
                .map_err(|e| BlueprintError::Invalid(format!("{}: {e}", wf.path)))?;
            let mut changed = false;
            if let Some(steps) = v
                .pointer_mut("/definition/steps")
                .and_then(|s| s.as_array_mut())
            {
                for step in steps {
                    let Some(kind) = step.get_mut("kind").and_then(|k| k.as_object_mut()) else {
                        continue;
                    };
                    if kind.get("type").and_then(|t| t.as_str()) != Some("code") {
                        continue;
                    }
                    let has_source = kind
                        .get("source")
                        .and_then(|s| s.as_str())
                        .is_some_and(|s| !s.trim().is_empty());
                    if has_source {
                        continue;
                    }
                    let module = kind
                        .get("module")
                        .and_then(|m| m.as_str())
                        .map(|m| m.trim_start_matches("./").to_string())
                        .ok_or_else(|| {
                            BlueprintError::Invalid(format!(
                                "{}: a code step needs `module` or `source`",
                                wf.path
                            ))
                        })?;
                    let source = tools.get(&module).ok_or_else(|| {
                        BlueprintError::Invalid(format!(
                            "{}: code module '{module}' is not in the blueprint's tools/",
                            wf.path
                        ))
                    })?;
                    kind.insert("source".into(), serde_json::Value::String(source.clone()));
                    changed = true;
                }
            }
            if changed {
                wf.content = serde_json::to_string_pretty(&v).unwrap_or_default();
            }
        }
        Ok(())
    }

    /// Structural checks: a name, parseable workflows, references that exist.
    pub fn validate(&self) -> Result<(), BlueprintError> {
        let slug_ok = !self.name.is_empty()
            && self
                .name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
        if !slug_ok {
            return Err(BlueprintError::Invalid(format!(
                "name '{}' must be lowercase letters, digits, '-' or '_'",
                self.name
            )));
        }
        for wf in &self.workflows {
            let v: serde_json::Value = serde_json::from_str(&wf.content)
                .map_err(|e| BlueprintError::Invalid(format!("{}: {e}", wf.path)))?;
            if v.get("agent_type").and_then(|t| t.as_str()) != Some("workflow_agent") {
                return Err(BlueprintError::Invalid(format!(
                    "{}: agent_type must be \"workflow_agent\"",
                    wf.path
                )));
            }
            if v.get("name").and_then(|n| n.as_str()).is_none() {
                return Err(BlueprintError::Invalid(format!(
                    "{}: missing name",
                    wf.path
                )));
            }
        }
        let agents = self.agent_names();
        let workflows = self.workflow_names();
        if let Some(ch) = &self.channel {
            if !agents.contains(&ch.agent) && !is_template(&ch.agent) {
                return Err(BlueprintError::Invalid(format!(
                    "channel.agent '{}' is not one of the blueprint's agents",
                    ch.agent
                )));
            }
            for w in &ch.workflows {
                if !workflows.contains(w) {
                    return Err(BlueprintError::Invalid(format!(
                        "channel.workflows: '{w}' is not one of the blueprint's workflows"
                    )));
                }
            }
        }
        if let Some(e) = &self.embed
            && !agents.contains(&e.agent) && !is_template(&e.agent) {
                return Err(BlueprintError::Invalid(format!(
                    "embed.agent '{}' is not one of the blueprint's agents",
                    e.agent
                )));
            }
        if let Some(t) = &self.test
            && !workflows.contains(&t.workflow) {
                return Err(BlueprintError::Invalid(format!(
                    "test.workflow '{}' is not one of the blueprint's workflows",
                    t.workflow
                )));
            }
        Ok(())
    }

    /// The values to use: given ones, then defaults. Errors on the first required one
    /// missing, carrying its question.
    pub fn resolve_params(
        &self,
        given: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>, BlueprintError> {
        let mut out = BTreeMap::new();
        for (name, p) in &self.params {
            let v = given
                .get(name)
                .filter(|v| !v.trim().is_empty())
                .cloned()
                .or_else(|| p.default.clone());
            match v {
                Some(v) => {
                    out.insert(name.clone(), v);
                }
                None if p.required => {
                    return Err(BlueprintError::MissingParam {
                        name: name.clone(),
                        ask: p.ask.clone(),
                    });
                }
                None => {
                    out.insert(name.clone(), String::new());
                }
            }
        }
        Ok(out)
    }

    /// Params a person still has to give.
    pub fn missing_params(
        &self,
        given: &BTreeMap<String, String>,
    ) -> Vec<(String, BlueprintParam)> {
        self.params
            .iter()
            .filter(|(n, p)| {
                p.required
                    && p.default.is_none()
                    && given.get(*n).is_none_or(|v| v.trim().is_empty())
            })
            .map(|(n, p)| (n.clone(), p.clone()))
            .collect()
    }

    /// A copy with every `{{params.x}}` replaced, in file contents and in the manifest's
    /// strings. Fails on a missing required param.
    pub fn render(&self, given: &BTreeMap<String, String>) -> Result<Blueprint, BlueprintError> {
        let values = self.resolve_params(given)?;
        let sub = |s: &str| render_template(s, &values);
        let files = |fs: &[BlueprintFile]| -> Vec<BlueprintFile> {
            fs.iter()
                .map(|f| BlueprintFile {
                    path: f.path.clone(),
                    content: sub(&f.content),
                })
                .collect()
        };
        let mut out = self.clone();
        out.agents = files(&self.agents);
        out.workflows = files(&self.workflows);
        out.skills = files(&self.skills);
        out.tools = files(&self.tools);
        if let Some(e) = &mut out.embed {
            e.origins = e.origins.iter().map(|o| sub(o)).collect();
            e.agent = sub(&e.agent);
        }
        if let Some(c) = &mut out.channel {
            c.agent = sub(&c.agent);
        }
        if let Some(t) = &mut out.test
            && let Ok(s) = serde_json::to_string(&t.input) {
                t.input = serde_json::from_str(&sub(&s)).unwrap_or(t.input.clone());
            }
        out.validate()?;
        Ok(out)
    }
}

fn is_template(s: &str) -> bool {
    s.contains("{{")
}

/// Replace `{{params.name}}` (spaces allowed inside the braces). Unknown names are left as
/// they are so a mistake shows up in the result instead of vanishing.
pub fn render_template(s: &str, values: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let key = after[..end].trim();
        match key.strip_prefix("params.").and_then(|k| values.get(k)) {
            Some(v) => out.push_str(v),
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// The `name` in an agent markdown file's TOML frontmatter (`+++` or `---` fenced).
pub fn agent_name_from_markdown(content: &str) -> Option<String> {
    let trimmed = content.trim_start();
    let fence = if trimmed.starts_with("+++") {
        "+++"
    } else if trimmed.starts_with("---") {
        "---"
    } else {
        return None;
    };
    let body = &trimmed[fence.len()..];
    let end = body.find(&format!("\n{fence}"))?;
    let fm = &body[..end];
    for line in fm.lines() {
        let l = line.trim();
        if let Some(v) = l.strip_prefix("name") {
            let v = v.trim_start();
            if let Some(v) = v.strip_prefix('=').or_else(|| v.strip_prefix(':')) {
                let v = v.trim().trim_matches('"').trim_matches('\'');
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

// ============================================================================
// Apply API (`POST /v1/blueprints/apply`), shared by the CLI and the server
// ============================================================================

/// What to apply: a full blueprint, or a sample by name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyBlueprintRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blueprint: Option<Blueprint>,
    /// A sample the server ships (`sre-agent`), when `blueprint` is not given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<String>,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    /// Only plan: nothing is created.
    #[serde(default)]
    pub dry_run: bool,
}

/// One thing applying created, updated or left as it was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedItem {
    /// `agent`, `workflow`, `skill`, `schedule`, `public_client`, `notify_target`
    pub kind: String,
    pub name: String,
    /// `created`, `updated`, `unchanged`, `planned`
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Something only a person can do, with the link to do it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingAction {
    /// Stable id within the install: `channel:telegram`, `connection:github`, `secret:X`,
    /// `embed:snippet`.
    pub id: String,
    /// `connect_channel`, `connect`, `secret`, `embed`
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub why: String,
    /// Where to do it (deep link, OAuth URL, dashboard page).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Text to paste (embed snippet), when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    pub required: bool,
    pub done: bool,
}

/// A param that is still needed, with its question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissingParam {
    pub name: String,
    pub ask: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyBlueprintResponse {
    /// Set once applied (not on a dry run or when params are missing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_id: Option<String>,
    pub blueprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `needs_params`, `planned`, `pending` (applied; a person still has to act), `ready`
    pub status: String,
    #[serde(default)]
    pub items: Vec<AppliedItem>,
    #[serde(default)]
    pub pending: Vec<PendingAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_params: Vec<MissingParam>,
    /// The first run to try once ready.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<BlueprintTest>,
    /// One line for a person.
    #[serde(default)]
    pub message: String,
}

/// One sample in the catalog (`GET /v1/blueprints/samples`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlueprintSummary {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub about: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub params: Vec<MissingParam>,
    pub agents: Vec<String>,
    pub workflows: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<ChannelKind>,
    pub embed: bool,
    pub connections: Vec<String>,
}

impl Blueprint {
    pub fn summary(&self) -> BlueprintSummary {
        BlueprintSummary {
            name: self.name.clone(),
            title: self.title.clone(),
            about: self.about.clone(),
            prompt: self.prompt.clone(),
            category: self.category.clone(),
            params: self
                .params
                .iter()
                .map(|(n, p)| MissingParam {
                    name: n.clone(),
                    ask: p.ask.clone(),
                    example: p.example.clone().or_else(|| p.default.clone()),
                })
                .collect(),
            agents: self.agent_names(),
            workflows: self.workflow_names(),
            channel: self.channel.as_ref().map(|c| c.kind),
            embed: self.embed.is_some(),
            connections: self
                .connections
                .iter()
                .map(|c| c.provider.clone())
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
name = "sre-agent"
title = "SRE agent"
about = "Ask it from Telegram; it checks your health URL and alerts you."
prompt = "Create an SRE agent for my API that I can ask from Telegram"
category = "engineering"

[params.service_url]
ask = "Which URL should I check?"
example = "https://api.acme.com/health"

[params.every]
ask = "How often?"
default = "*/5"

[channel]
kind = "telegram"
agent = "acme_sre"
workflows = ["acme_health"]

[[connections]]
provider = "github"
required = false
why = "read deploys"

[test]
workflow = "acme_health"
input = { force_alert = true }
expect = "a test alert arrives on Telegram"
"#;

    fn files() -> Vec<(String, String)> {
        vec![
            (
                "agents/acme_sre.md".into(),
                "---\nname = \"acme_sre\"\n---\nWatch {{params.service_url}}".into(),
            ),
            (
                "workflows/acme_health.json".into(),
                serde_json::json!({
                    "agent_type": "workflow_agent",
                    "name": "acme_health",
                    "description": "",
                    "definition": {"id": "h", "steps": [
                        {"id": "check", "label": "c",
                         "kind": {"type": "code", "module": "tools/check.ts"},
                         "input": {"url": "{{params.service_url}}"}}
                    ]}
                })
                .to_string(),
            ),
            (
                "tools/check.ts".into(),
                "export default () => ({ok: true})".into(),
            ),
            ("README.md".into(), "ignored".into()),
        ]
    }

    #[test]
    fn loads_inlines_modules_and_renders_params() {
        let bp = Blueprint::from_parts(MANIFEST, files()).unwrap();
        assert_eq!(bp.agent_names(), vec!["acme_sre"]);
        assert_eq!(bp.workflow_names(), vec!["acme_health"]);
        assert!(bp.workflows[0].content.contains("export default"));
        assert_eq!(bp.test.as_ref().unwrap().input["force_alert"], true);

        let missing = bp.missing_params(&BTreeMap::new());
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].0, "service_url");
        assert_eq!(
            bp.render(&BTreeMap::new()).unwrap_err(),
            BlueprintError::MissingParam {
                name: "service_url".into(),
                ask: "Which URL should I check?".into()
            }
        );

        let mut given = BTreeMap::new();
        given.insert(
            "service_url".to_string(),
            "https://x.dev/health".to_string(),
        );
        let r = bp.render(&given).unwrap();
        assert!(r.agents[0].content.ends_with("Watch https://x.dev/health"));
        assert!(r.workflows[0].content.contains("https://x.dev/health"));
    }

    #[test]
    fn rejects_a_channel_agent_that_is_not_in_the_blueprint() {
        let m = MANIFEST.replace("agent = \"acme_sre\"", "agent = \"nobody\"");
        assert!(matches!(
            Blueprint::from_parts(&m, files()),
            Err(BlueprintError::Invalid(_))
        ));
    }

    #[test]
    fn rejects_a_missing_code_module() {
        let mut f = files();
        f.retain(|(p, _)| p != "tools/check.ts");
        let err = Blueprint::from_parts(MANIFEST, f).unwrap_err();
        assert!(err.to_string().contains("tools/check.ts"), "{err}");
    }

    #[test]
    fn render_leaves_unknown_placeholders() {
        let mut v = BTreeMap::new();
        v.insert("a".to_string(), "1".to_string());
        assert_eq!(
            render_template("{{params.a}} {{ params.a }} {{params.b}} {input.x}", &v),
            "1 1 {{params.b}} {input.x}"
        );
    }

    #[test]
    fn agent_name_from_frontmatter() {
        assert_eq!(
            agent_name_from_markdown("+++\nname = \"x\"\n+++\nhi").as_deref(),
            Some("x")
        );
        assert_eq!(agent_name_from_markdown("no frontmatter"), None);
    }

    /// Every folder under `samples/` with a `blueprint.toml` loads, validates and renders
    /// with its example values.
    #[test]
    fn shipped_samples_load_and_render() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../samples");
        let mut seen = 0;
        for entry in std::fs::read_dir(&root).unwrap().flatten() {
            let dir = entry.path();
            if !dir.join(BLUEPRINT_FILE).is_file() {
                continue;
            }
            seen += 1;
            let bp = Blueprint::load_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
            assert!(bp.prompt.is_some(), "{}: needs a prompt", bp.name);
            let given: BTreeMap<String, String> = bp
                .params
                .iter()
                .map(|(n, p)| {
                    (
                        n.clone(),
                        p.example
                            .clone()
                            .or_else(|| p.default.clone())
                            .unwrap_or_else(|| "x".into()),
                    )
                })
                .collect();
            let r = bp.render(&given).unwrap();
            for f in r.agents.iter().chain(&r.workflows).chain(&r.skills) {
                assert!(
                    !f.content.contains("{{params."),
                    "{}: unfilled param in {}",
                    bp.name,
                    f.path
                );
            }
            for wf in &r.workflows {
                let v: serde_json::Value = serde_json::from_str(&wf.content).unwrap();
                assert!(
                    v.pointer("/definition/steps").is_some(),
                    "{}: {} has no steps",
                    bp.name,
                    wf.path
                );
            }
        }
        assert!(seen >= 2, "expected samples under {}", root.display());
    }
}
