//! Local TypeScript scripts as external tools.
//!
//! On start the CLI scans `.distri/tools/*.ts` (project) and `~/.distri/tools/*.ts` (user),
//! reads each script's `export const tool = defineTool({...})` with `@distri/tool`'s extractor,
//! and registers a [`ToolDefinition`] plus a handler on the [`ExternalToolRegistry`] next to the
//! built-ins. A call runs the script with `deno run` under the frozen script contract (one JSON
//! document on stdin, one on stdout, logs on stderr, non-zero exit is a failure) and only the
//! permissions it was granted (see [`permissions`]).
//!
//! Before Deno starts, the call's input is validated against the tool's `parameters`. While it
//! runs, `ctx.progress` / `ctx.log` / `ctx.warn` and plain stderr lines go out as
//! [`LocalToolEvent`]s keyed by the tool call; dropping the call (a cancelled run) sends the
//! script SIGTERM, so `ctx.signal` aborts. Permission prompts go through the [`Broker`].

pub mod permissions;
pub mod runner;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use distri::ExternalToolRegistry;
use distri_types::script_tool::ToolExport;
use distri_types::{AgentEvent, Part, ToolCall, ToolDefinition, ToolResponse};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use permissions::{DistriToml, GrantStore, Permissions, Remembered, ToolSettings};
pub use runner::{CallOutput, Deno, EventSink, ExtractFailure, ScriptToolError};

use crate::interact::{Interaction, Reply, SharedBroker};
use crate::local_events::{LocalEvents, LocalToolEvent};
use permissions::parse_frontmatter;
use runner::ResolvedGrant;

/// Where script tools come from and where the CLI keeps its state.
#[derive(Debug, Clone)]
pub struct ScriptToolsConfig {
    /// The project root: `.distri/tools/` lives here, relative `read` grants resolve here.
    pub project_root: PathBuf,
    /// The project's `distri.toml` (may not exist).
    pub distri_toml: PathBuf,
    /// `~/.distri/tools/`; `None` to skip user tools.
    pub user_tools_dir: Option<PathBuf>,
    /// `~/.distri`: `run/` (the harness), `cache/` (Deno's cache, the extract cache) and
    /// `tool_grants.json`.
    pub distri_home: PathBuf,
    /// The `deno` binary; `None` when it is not installed.
    pub deno: Option<PathBuf>,
}

impl ScriptToolsConfig {
    /// The defaults for a project: `~/.distri` and `deno` from `DISTRI_DENO` or `PATH`.
    pub fn for_project(project_root: &Path, distri_toml: Option<PathBuf>) -> anyhow::Result<Self> {
        let home = dirs::home_dir().context("could not resolve the home directory")?;
        let distri_home = home.join(".distri");
        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.to_path_buf());
        Ok(Self {
            distri_toml: distri_toml.unwrap_or_else(|| project_root.join("distri.toml")),
            user_tools_dir: Some(distri_home.join("tools")),
            project_root,
            distri_home,
            deno: runner::locate_deno(),
        })
    }

    pub fn project_tools_dir(&self) -> PathBuf {
        self.project_root.join(".distri").join("tools")
    }
}

/// Where a script was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolScope {
    Project,
    User,
}

impl std::fmt::Display for ToolScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ToolScope::Project => "project",
            ToolScope::User => "user",
        })
    }
}

/// A script with a `tool` export.
#[derive(Debug, Clone)]
pub struct ScriptTool {
    pub export: ToolExport,
    /// Absolute, canonical.
    pub path: PathBuf,
    pub scope: ToolScope,
    /// What its frontmatter asks for.
    pub requested: Permissions,
    /// Its frontmatter's `[tool] timeout`, in seconds.
    pub timeout: Option<u64>,
}

impl ScriptTool {
    pub fn name(&self) -> &str {
        &self.export.name
    }

    pub fn definition(&self) -> ToolDefinition {
        self.export.clone().into()
    }

    fn validator(&self) -> Result<jsonschema::Validator, ScriptToolError> {
        jsonschema::validator_for(&self.export.parameters).map_err(|e| ScriptToolError::BadSchema {
            tool: self.name().to_string(),
            detail: e.to_string(),
        })
    }

    /// `Ok` when `parameters` is a JSON Schema the host can validate input with.
    pub fn check_schema(&self) -> Result<(), ScriptToolError> {
        self.validator().map(|_| ())
    }

    /// Validate a call's input against `parameters`. The error names every mismatch with its
    /// JSON pointer: `/id: 17 is not of type "string"`.
    pub fn validate_input(&self, input: &serde_json::Value) -> Result<(), ScriptToolError> {
        let validator = self.validator()?;
        let problems: Vec<String> = validator
            .iter_errors(input)
            .map(|e| {
                let at = e.instance_path().to_string();
                if at.is_empty() {
                    e.to_string()
                } else {
                    format!("{at}: {e}")
                }
            })
            .collect();
        if problems.is_empty() {
            Ok(())
        } else {
            Err(ScriptToolError::InvalidInput {
                tool: self.name().to_string(),
                detail: problems.join("; "),
            })
        }
    }
}

/// A script that is not registered, and why.
#[derive(Debug, Clone)]
pub struct ScriptProblem {
    pub path: PathBuf,
    pub message: String,
    /// The same source failed the same way before (the extract cache): print it short.
    pub cached: bool,
}

impl ScriptProblem {
    fn new(path: PathBuf, message: String) -> Self {
        Self {
            path,
            message,
            cached: false,
        }
    }

    /// The first line of the message, for repeat notices.
    pub fn summary(&self) -> &str {
        self.message.lines().next().unwrap_or_default()
    }
}

/// What one scan found.
#[derive(Debug, Clone, Default)]
pub struct Discovery {
    pub tools: Vec<ScriptTool>,
    /// Scripts that failed to load or conflict with another tool.
    pub problems: Vec<ScriptProblem>,
    /// Scripts without a `tool` export.
    pub not_tools: Vec<PathBuf>,
    /// Scripts not read because Deno is not installed.
    pub skipped_no_deno: Vec<PathBuf>,
}

impl Discovery {
    pub fn find(&self, name: &str) -> Option<&ScriptTool> {
        self.tools.iter().find(|t| t.name() == name).or_else(|| {
            self.tools
                .iter()
                .find(|t| t.path.file_stem().and_then(|s| s.to_str()) == Some(name))
        })
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(ScriptTool::definition).collect()
    }

    /// The one-time notice when Deno is missing: names every script tool that was skipped.
    pub fn deno_missing_notice(&self) -> Option<String> {
        if self.skipped_no_deno.is_empty() {
            return None;
        }
        let names: Vec<String> = self
            .skipped_no_deno
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        Some(format!(
            "Deno is not installed, so these script tools were skipped: {}. Install it from https://deno.com (or set DISTRI_DENO to the binary); built-in tools still work.",
            names.join(", ")
        ))
    }
}

/// `~/.distri/cache/script_tools.json`: the last extraction of each script, by content hash.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ExtractCache {
    #[serde(default)]
    entries: BTreeMap<String, ExtractCacheEntry>,
}

/// One script's last extraction. The hash covers the script and its sibling `*.ts` files (what
/// it may import), so editing `./lib.ts` re-extracts.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExtractCacheEntry {
    sha256: String,
    tool: Option<ToolExport>,
    /// The import error, when the extraction failed (the same source fails the same way).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Discovers, grants and runs script tools for one project.
pub struct ScriptToolHost {
    cfg: ScriptToolsConfig,
    deno: Option<Deno>,
    grants: GrantStore,
    broker: SharedBroker,
    /// One permission prompt at a time.
    prompt_lock: tokio::sync::Mutex<()>,
}

/// How one `distri tools check` of a script went.
#[derive(Debug, Clone)]
pub struct CheckReport {
    /// The tool name, or the script's file stem when it did not load.
    pub name: String,
    pub path: PathBuf,
    /// `Ok` when `deno check` and the schema pass; otherwise the first error lines.
    pub result: Result<(), String>,
}

impl ScriptToolHost {
    pub fn new(cfg: ScriptToolsConfig, broker: SharedBroker) -> anyhow::Result<Self> {
        let deno = match &cfg.deno {
            Some(binary) => Some(Deno::prepare(
                binary.clone(),
                &cfg.distri_home.join("run"),
                cfg.distri_home.join("cache").join("deno"),
            )?),
            None => None,
        };
        let grants = GrantStore::new(cfg.distri_home.join("tool_grants.json"));
        Ok(Self {
            cfg,
            deno,
            grants,
            broker,
            prompt_lock: tokio::sync::Mutex::new(()),
        })
    }

    pub fn config(&self) -> &ScriptToolsConfig {
        &self.cfg
    }

    pub fn grants(&self) -> &GrantStore {
        &self.grants
    }

    fn extract_cache_path(&self) -> PathBuf {
        self.cfg.distri_home.join("cache").join("script_tools.json")
    }

    /// Scan both folders. Project tools shadow user tools of the same name; a name in
    /// `reserved` (the built-ins) is a problem, never an override.
    pub async fn discover(&self, reserved: &BTreeSet<String>) -> Discovery {
        let mut found: Vec<(ToolScope, PathBuf)> = list_scripts(&self.cfg.project_tools_dir())
            .into_iter()
            .map(|p| (ToolScope::Project, p))
            .collect();
        if let Some(dir) = &self.cfg.user_tools_dir {
            found.extend(list_scripts(dir).into_iter().map(|p| (ToolScope::User, p)));
        }
        let mut out = Discovery::default();
        let Some(deno) = &self.deno else {
            out.skipped_no_deno = found.into_iter().map(|(_, p)| p).collect();
            return out;
        };

        let mut cache: ExtractCache = std::fs::read_to_string(self.extract_cache_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut cache_dirty = false;

        // Read every script's source; extract the ones the cache does not know, concurrently.
        let mut loaded: Vec<(ToolScope, PathBuf, String, Permissions, Option<u64>)> = Vec::new();
        let mut jobs = tokio::task::JoinSet::new();
        let mut folder_digests: BTreeMap<PathBuf, String> = BTreeMap::new();
        for (scope, path) in found {
            let source = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    out.problems
                        .push(ScriptProblem::new(path, format!("unreadable: {e}")));
                    continue;
                }
            };
            let fm = match parse_frontmatter(&String::from_utf8_lossy(&source)) {
                Ok(fm) => fm,
                Err(e) => {
                    out.problems
                        .push(ScriptProblem::new(path, format!("{e:#}")));
                    continue;
                }
            };
            let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
            let siblings = folder_digests
                .entry(folder.clone())
                .or_insert_with(|| folder_digest(&folder));
            let mut hasher = Sha256::new();
            hasher.update(&source);
            hasher.update(siblings.as_bytes());
            let sha = hex::encode(hasher.finalize());
            let key = path.display().to_string();
            if cache.entries.get(&key).is_none_or(|e| e.sha256 != sha) {
                let deno = deno.clone();
                let cwd = self.cfg.project_root.clone();
                let p = path.clone();
                let sha = sha.clone();
                jobs.spawn(async move {
                    let r = deno.extract(&p, &cwd).await;
                    (p, sha, r)
                });
            }
            loaded.push((scope, path, sha, fm.permissions, fm.tool.timeout));
        }
        // Failures this scan that are not cached (a timeout, Deno not starting).
        let mut failed: BTreeMap<PathBuf, String> = BTreeMap::new();
        // Failures this scan found fresh; cached ones are reported short.
        let mut fresh: BTreeSet<PathBuf> = BTreeSet::new();
        while let Some(joined) = jobs.join_next().await {
            let Ok((path, sha, result)) = joined else {
                continue;
            };
            let entry = match result {
                Ok(tool) => ExtractCacheEntry {
                    sha256: sha,
                    tool,
                    error: None,
                },
                Err(ExtractFailure::Script(message)) => {
                    fresh.insert(path.clone());
                    ExtractCacheEntry {
                        sha256: sha,
                        tool: None,
                        error: Some(message),
                    }
                }
                Err(e) => {
                    failed.insert(path, format!("{e:#}"));
                    continue;
                }
            };
            cache.entries.insert(path.display().to_string(), entry);
            cache_dirty = true;
        }
        if cache_dirty {
            if let Ok(body) = serde_json::to_vec_pretty(&cache) {
                if let Err(e) = write_atomic(&self.extract_cache_path(), &body) {
                    tracing::warn!("script tools: could not write the extract cache: {e:#}");
                }
            }
        }

        let mut by_name: BTreeMap<String, usize> = BTreeMap::new();
        for (scope, path, sha, requested, timeout) in loaded {
            if let Some(message) = failed.remove(&path) {
                out.problems.push(ScriptProblem::new(path, message));
                continue;
            }
            let Some(entry) = cache
                .entries
                .get(&path.display().to_string())
                .filter(|e| e.sha256 == sha)
            else {
                out.not_tools.push(path);
                continue;
            };
            if let Some(message) = &entry.error {
                out.problems.push(ScriptProblem {
                    cached: !fresh.contains(&path),
                    message: message.clone(),
                    path,
                });
                continue;
            }
            let Some(export) = entry.tool.clone() else {
                out.not_tools.push(path);
                continue;
            };
            if reserved.contains(&export.name) {
                out.problems.push(ScriptProblem::new(
                    path,
                    format!("tool '{}' conflicts with a built-in tool", export.name),
                ));
                continue;
            }
            if let Some(&i) = by_name.get(&export.name) {
                let winner = &out.tools[i];
                let message = if winner.scope == ToolScope::Project && scope == ToolScope::User {
                    format!(
                        "tool '{}' is shadowed by the project's {}",
                        export.name,
                        winner.path.display()
                    )
                } else {
                    format!(
                        "tool '{}' is also defined by {}",
                        export.name,
                        winner.path.display()
                    )
                };
                out.problems.push(ScriptProblem::new(path, message));
                continue;
            }
            by_name.insert(export.name.clone(), out.tools.len());
            out.tools.push(ScriptTool {
                export,
                path,
                scope,
                requested,
                timeout,
            });
        }
        out
    }

    /// The read paths and net hosts `tool` gets for this call: its own folder, what
    /// `distri.toml` grants, and what its frontmatter asks for when that is (or now gets)
    /// approved for this project.
    pub async fn resolve_grant(&self, tool: &ScriptTool) -> anyhow::Result<ResolvedGrant> {
        let configured = DistriToml::load(&self.cfg.distri_toml)?.grant_for(tool.name());
        let extra = tool.requested.minus(&configured);
        let mut granted = configured.clone();
        if !extra.is_empty() {
            let _one_prompt_at_a_time = self.prompt_lock.lock().await;
            let project = &self.cfg.project_root;
            let allowed = match self
                .grants
                .lookup(project, tool.name(), &tool.path, &extra)?
            {
                Remembered::Allowed => true,
                Remembered::Denied => false,
                Remembered::Unknown => {
                    let reply = self
                        .broker
                        .ask(Interaction::Grant {
                            tool: tool.name().to_string(),
                            script: tool.path.clone(),
                            request: extra.describe(),
                        })
                        .await;
                    match reply {
                        // "Always": stored for this project.
                        Reply::Allow { remember: true } => {
                            self.grants
                                .remember(project, tool.name(), &tool.path, &extra, true)?;
                            true
                        }
                        // "Once": this call only.
                        Reply::Allow { remember: false } => true,
                        // "No": remembered for this exact request.
                        Reply::Deny { .. } => {
                            self.grants.remember(
                                project,
                                tool.name(),
                                &tool.path,
                                &extra,
                                false,
                            )?;
                            false
                        }
                        // Nobody to ask (or an answer that is not a yes/no): not this call,
                        // not remembered.
                        Reply::Unavailable | Reply::Text(_) => {
                            tracing::warn!(
                                "script tool '{}' asks for {} but nobody could approve it; running without",
                                tool.name(),
                                extra.describe()
                            );
                            false
                        }
                    }
                }
            };
            if allowed {
                granted = granted.union(&extra);
            }
        }

        let folder = tool
            .path
            .parent()
            .context("script has no parent folder")?
            .to_path_buf();
        let mut read = vec![folder];
        for p in &granted.read {
            let path = self.cfg.project_root.join(p);
            read.push(path.canonicalize().unwrap_or(path));
        }
        Ok(ResolvedGrant {
            read,
            net: granted.net.into_iter().collect(),
        })
    }

    /// How long one call of `tool` may run: `[tools.<name>] timeout` in `distri.toml`, else
    /// the script's `[tool] timeout`, else [`runner::CALL_TIMEOUT`].
    pub fn timeout_for(&self, tool: &ScriptTool) -> anyhow::Result<Duration> {
        let configured = DistriToml::load(&self.cfg.distri_toml)?.timeout_for(tool.name());
        Ok(configured
            .or(tool.timeout)
            .map(Duration::from_secs)
            .unwrap_or(runner::CALL_TIMEOUT))
    }

    /// Run `tool` with `input`. The input is validated against the tool's schema before Deno
    /// starts; progress and log lines go to `sink` while it runs. Dropping the future
    /// terminates the script (SIGTERM, then SIGKILL).
    pub async fn call(
        &self,
        tool: &ScriptTool,
        input: &serde_json::Value,
        sink: Option<EventSink>,
    ) -> Result<CallOutput, ScriptToolError> {
        tool.validate_input(input)?;
        let deno = self.deno.as_ref().ok_or_else(|| {
            ScriptToolError::Io(anyhow::anyhow!(
                "Deno is not installed; script tool '{}' cannot run (https://deno.com)",
                tool.name()
            ))
        })?;
        let timeout = self.timeout_for(tool)?;
        let grant = self.resolve_grant(tool).await?;
        let run = deno.call_run(&tool.path, &self.cfg.project_root, &grant);
        deno.call(tool.name(), &run, input, timeout, sink).await
    }

    /// `deno check` (with the `@distri/tool` import map) and the schema of every discovered
    /// tool, or only of `name` (a tool name or script file stem). Scripts that did not load are
    /// reported with their load error.
    pub async fn check(&self, discovery: &Discovery, name: Option<&str>) -> Vec<CheckReport> {
        let stem = |p: &Path| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string()
        };
        let wanted =
            |tool_name: &str, path: &Path| name.is_none_or(|n| n == tool_name || n == stem(path));
        let mut out = Vec::new();
        for tool in discovery.tools.iter().filter(|t| wanted(t.name(), &t.path)) {
            let result = match &self.deno {
                None => Err("Deno is not installed (https://deno.com)".to_string()),
                Some(deno) => match deno.check(&tool.path, &self.cfg.project_root).await {
                    Err(e) => Err(format!("{e:#}")),
                    Ok(Err(lines)) => Err(lines),
                    Ok(Ok(())) => tool.check_schema().map_err(|e| e.to_string()),
                },
            };
            out.push(CheckReport {
                name: tool.name().to_string(),
                path: tool.path.clone(),
                result,
            });
        }
        for problem in &discovery.problems {
            let name = stem(&problem.path);
            if !wanted(&name, &problem.path) {
                continue;
            }
            out.push(CheckReport {
                name,
                path: problem.path.clone(),
                result: Err(problem.message.clone()),
            });
        }
        out
    }
}

/// Register a handler for every discovered tool under `"*"` (like the built-ins) and return
/// their definitions, for `inject_external_tools`. A call's progress and log lines go out on
/// `events`, keyed by its `tool_call_id`.
pub fn register(
    host: &Arc<ScriptToolHost>,
    registry: &ExternalToolRegistry,
    discovery: &Discovery,
    events: LocalEvents,
) -> Vec<ToolDefinition> {
    for tool in &discovery.tools {
        let host = host.clone();
        let tool = tool.clone();
        let events = events.clone();
        registry.register(
            "*",
            tool.name().to_string(),
            move |call: ToolCall, _event: AgentEvent| {
                let host = host.clone();
                let tool = tool.clone();
                let events = events.clone();
                async move {
                    let sink: EventSink = {
                        let tool_call_id = call.tool_call_id.clone();
                        let tool_name = call.tool_name.clone();
                        Arc::new(move |kind| {
                            events.emit(LocalToolEvent {
                                tool_call_id: tool_call_id.clone(),
                                tool_name: tool_name.clone(),
                                kind,
                            })
                        })
                    };
                    let out = host.call(&tool, &call.input, Some(sink)).await?;
                    if !out.stderr.trim().is_empty() {
                        tracing::debug!(
                            tool = tool.name(),
                            "script stderr:\n{}",
                            out.stderr.trim()
                        );
                    }
                    Ok(ToolResponse::from_parts(
                        call.tool_call_id,
                        call.tool_name,
                        vec![Part::Data(out.value)],
                    ))
                }
            },
        );
    }
    discovery.definitions()
}

/// A digest of every `*.ts` file directly in `folder` (name and content), sorted: what a
/// script there may import with `./x.ts`.
fn folder_digest(folder: &Path) -> String {
    let mut hasher = Sha256::new();
    for path in list_scripts(folder) {
        hasher.update(
            path.file_name()
                .map(|n| n.as_encoded_bytes())
                .unwrap_or_default(),
        );
        hasher.update([0]);
        if let Ok(bytes) = std::fs::read(&path) {
            hasher.update(Sha256::digest(&bytes));
        }
    }
    hex::encode(hasher.finalize())
}

/// `*.ts` directly under `dir`, sorted, canonical.
fn list_scripts(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("ts") && p.is_file())
        .map(|p| p.canonicalize().unwrap_or(p))
        .collect();
    out.sort();
    out
}

/// Write via a temp file and a rename, so a concurrent reader never sees half a file.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().context("path has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("rename to {}", path.display()))
}

/// A valid tool name: a letter or `_`, then letters, digits, `_` or `-`.
pub fn validate_name(name: &str) -> anyhow::Result<()> {
    let mut chars = name.chars();
    let ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        anyhow::bail!(
            "invalid tool name '{name}': use letters, digits, '_' and '-', starting with a letter"
        )
    }
}

/// Write `<dir>/<name>.ts` from the template. Refuses to overwrite.
pub fn scaffold(dir: &Path, name: &str) -> anyhow::Result<PathBuf> {
    validate_name(name)?;
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(format!("{name}.ts"));
    if path.exists() {
        anyhow::bail!("{} already exists", path.display());
    }
    std::fs::write(&path, template(name)).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn template(name: &str) -> String {
    format!(
        r#"// ---
// # What this tool may touch. Nothing is granted unless listed here (approved once per
// # project) or under [tools.{name}] in distri.toml. Env and subprocesses are never granted.
// [permissions]
// net = []
// read = []
// # How long one call may run, in seconds (default 120).
// [tool]
// timeout = 120
// ---
import {{ defineTool, s }} from "jsr:@distri/tool";

export const tool = defineTool({{
  name: "{name}",
  description: "Describe what {name} does",
  // The input schema: the agent sees it, and the CLI checks every call against it.
  input: s.object({{ input: s.string({{ description: "What to work on" }}) }}),
  // One JSON document in, one out. ctx.progress / ctx.log / ctx.warn show up with the call;
  // ctx.signal aborts when the run is cancelled.
  async run({{ input }}, ctx) {{
    ctx.log(`working on ${{input}}`);
    return {{ ok: true, input }};
  }},
}});
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_validated() {
        assert!(validate_name("lookup_order").is_ok());
        assert!(validate_name("a-b2").is_ok());
        assert!(validate_name("2a").is_err());
        assert!(validate_name("../x").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn scaffold_writes_a_template_with_empty_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = scaffold(dir.path(), "lookup_order").unwrap();
        let src = std::fs::read_to_string(&path).unwrap();
        assert!(src.contains("name: \"lookup_order\""));
        let fm = parse_frontmatter(&src).unwrap();
        assert!(fm.permissions.is_empty());
        assert_eq!(fm.tool.timeout, Some(120));
        assert!(scaffold(dir.path(), "lookup_order").is_err());
    }
}
