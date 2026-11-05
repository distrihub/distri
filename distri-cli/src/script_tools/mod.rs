//! Local TypeScript scripts as external tools.
//!
//! On start the CLI scans `.distri/tools/*.ts` (project) and `~/.distri/tools/*.ts` (user),
//! reads each script's `export const tool = defineTool({...})` with `@distri/tool`'s extractor,
//! and registers a [`ToolDefinition`] plus a handler on the [`ExternalToolRegistry`] next to the
//! built-ins. A call runs the script with `deno run` under the frozen script contract (one JSON
//! document on stdin, one on stdout, logs on stderr, non-zero exit is a failure) and only the
//! permissions it was granted (see [`permissions`]).

pub mod permissions;
pub mod runner;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use distri::ExternalToolRegistry;
use distri_types::script_tool::ToolExport;
use distri_types::{AgentEvent, Part, ToolCall, ToolDefinition, ToolResponse};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use permissions::{
    Approver, DistriToml, FixedApprover, GrantStore, Permissions, Remembered, TerminalApprover,
};
pub use runner::{CallOutput, Deno, ScriptToolError};

use permissions::{parse_frontmatter, ApprovalRequest};
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
}

impl ScriptTool {
    pub fn name(&self) -> &str {
        &self.export.name
    }

    pub fn definition(&self) -> ToolDefinition {
        self.export.clone().into()
    }
}

/// A script that is not registered, and why.
#[derive(Debug, Clone)]
pub struct ScriptProblem {
    pub path: PathBuf,
    pub message: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExtractCacheEntry {
    sha256: String,
    tool: Option<ToolExport>,
}

/// Discovers, grants and runs script tools for one project.
pub struct ScriptToolHost {
    cfg: ScriptToolsConfig,
    deno: Option<Deno>,
    grants: GrantStore,
    approver: Arc<dyn Approver>,
    /// One permission prompt at a time.
    prompt_lock: tokio::sync::Mutex<()>,
}

impl ScriptToolHost {
    pub fn new(cfg: ScriptToolsConfig, approver: Arc<dyn Approver>) -> anyhow::Result<Self> {
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
            approver,
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
        let mut loaded: Vec<(ToolScope, PathBuf, String, Permissions)> = Vec::new();
        let mut jobs = tokio::task::JoinSet::new();
        for (scope, path) in found {
            let source = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    out.problems.push(ScriptProblem {
                        path,
                        message: format!("unreadable: {e}"),
                    });
                    continue;
                }
            };
            let requested = match parse_frontmatter(&String::from_utf8_lossy(&source)) {
                Ok(fm) => fm.permissions,
                Err(e) => {
                    out.problems.push(ScriptProblem {
                        path,
                        message: format!("{e:#}"),
                    });
                    continue;
                }
            };
            let sha = hex::encode(Sha256::digest(&source));
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
            loaded.push((scope, path, sha, requested));
        }
        let mut failed: BTreeMap<PathBuf, String> = BTreeMap::new();
        while let Some(joined) = jobs.join_next().await {
            let Ok((path, sha, result)) = joined else {
                continue;
            };
            match result {
                Ok(tool) => {
                    cache.entries.insert(
                        path.display().to_string(),
                        ExtractCacheEntry { sha256: sha, tool },
                    );
                    cache_dirty = true;
                }
                Err(e) => {
                    failed.insert(path, format!("{e:#}"));
                }
            }
        }
        if cache_dirty {
            if let Ok(body) = serde_json::to_vec_pretty(&cache) {
                if let Err(e) = write_atomic(&self.extract_cache_path(), &body) {
                    tracing::warn!("script tools: could not write the extract cache: {e:#}");
                }
            }
        }

        let mut by_name: BTreeMap<String, usize> = BTreeMap::new();
        for (scope, path, sha, requested) in loaded {
            if let Some(message) = failed.remove(&path) {
                out.problems.push(ScriptProblem { path, message });
                continue;
            }
            let export = cache
                .entries
                .get(&path.display().to_string())
                .filter(|e| e.sha256 == sha)
                .and_then(|e| e.tool.clone());
            let Some(export) = export else {
                out.not_tools.push(path);
                continue;
            };
            if reserved.contains(&export.name) {
                out.problems.push(ScriptProblem {
                    message: format!("tool '{}' conflicts with a built-in tool", export.name),
                    path,
                });
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
                out.problems.push(ScriptProblem { path, message });
                continue;
            }
            by_name.insert(export.name.clone(), out.tools.len());
            out.tools.push(ScriptTool {
                export,
                path,
                scope,
                requested,
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
                    let req = ApprovalRequest {
                        project: project.clone(),
                        tool: tool.name().to_string(),
                        script: tool.path.clone(),
                        request: extra.clone(),
                    };
                    let approver = self.approver.clone();
                    let answer = tokio::task::spawn_blocking(move || approver.approve(&req))
                        .await
                        .context("permission prompt")?;
                    match answer {
                        Some(allowed) => {
                            self.grants.remember(
                                project,
                                tool.name(),
                                &tool.path,
                                &extra,
                                allowed,
                            )?;
                            allowed
                        }
                        None => {
                            tracing::warn!(
                                "script tool '{}' asks for {} but there is no terminal to approve it; running without",
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

    /// Run `tool` with `input`.
    pub async fn call(
        &self,
        tool: &ScriptTool,
        input: &serde_json::Value,
    ) -> Result<CallOutput, ScriptToolError> {
        let deno = self.deno.as_ref().ok_or_else(|| {
            ScriptToolError::Io(anyhow::anyhow!(
                "Deno is not installed; script tool '{}' cannot run (https://deno.com)",
                tool.name()
            ))
        })?;
        let grant = self.resolve_grant(tool).await?;
        let run = deno.call_run(&tool.path, &self.cfg.project_root, &grant);
        deno.call(tool.name(), &run, input).await
    }
}

/// Register a handler for every discovered tool under `"*"` (like the built-ins) and return
/// their definitions, for `inject_external_tools`.
pub fn register(
    host: &Arc<ScriptToolHost>,
    registry: &ExternalToolRegistry,
    discovery: &Discovery,
) -> Vec<ToolDefinition> {
    for tool in &discovery.tools {
        let host = host.clone();
        let tool = tool.clone();
        registry.register(
            "*",
            tool.name().to_string(),
            move |call: ToolCall, _event: AgentEvent| {
                let host = host.clone();
                let tool = tool.clone();
                async move {
                    let out = host.call(&tool, &call.input).await?;
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
// ---
import {{ defineTool }} from "jsr:@distri/tool";

export const tool = defineTool({{
  name: "{name}",
  description: "Describe what {name} does",
  parameters: {{
    type: "object",
    properties: {{ input: {{ type: "string" }} }},
    required: ["input"],
  }},
}});

// One JSON document in, one out. Log with console.error (stderr); stdout is the result.
export default async function ({{ input }}: {{ input: string }}) {{
  return {{ ok: true, input }};
}}
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
        assert!(parse_frontmatter(&src).unwrap().permissions.is_empty());
        assert!(scaffold(dir.path(), "lookup_order").is_err());
    }
}
