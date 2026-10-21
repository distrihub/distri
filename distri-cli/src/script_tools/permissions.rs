//! What a script tool may touch. Permissions are declared, never inherited:
//!
//! - the script **requests** `net` hosts and `read` paths in its frontmatter (a TOML block in
//!   leading `//` comments between `// ---` fences);
//! - the project **grants** them without asking under `[tools.<name>]` in `distri.toml`;
//! - anything requested but not granted there is asked about once per project, and the answer is
//!   remembered in `~/.distri/tool_grants.json`.
//!
//! Env, run, ffi, sys and write are never granted.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The two permissions a script tool can ask for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// Hosts (`host` or `host:port`) for `--allow-net`.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub net: BTreeSet<String>,
    /// Paths for `--allow-read`; relative paths resolve against the project root.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub read: BTreeSet<String>,
}

impl Permissions {
    pub fn is_empty(&self) -> bool {
        self.net.is_empty() && self.read.is_empty()
    }

    /// What `self` asks for that `granted` does not cover.
    pub fn minus(&self, granted: &Permissions) -> Permissions {
        Permissions {
            net: self.net.difference(&granted.net).cloned().collect(),
            read: self.read.difference(&granted.read).cloned().collect(),
        }
    }

    pub fn union(&self, other: &Permissions) -> Permissions {
        Permissions {
            net: self.net.union(&other.net).cloned().collect(),
            read: self.read.union(&other.read).cloned().collect(),
        }
    }

    /// `true` when `self` grants everything `other` asks for.
    pub fn covers(&self, other: &Permissions) -> bool {
        other.minus(self).is_empty()
    }

    /// One line for prompts and listings: `net: a, b; read: ./x`.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if !self.net.is_empty() {
            parts.push(format!(
                "net: {}",
                self.net.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        if !self.read.is_empty() {
            parts.push(format!(
                "read: {}",
                self.read.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join("; ")
        }
    }
}

/// The TOML between the `// ---` fences at the top of a script.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frontmatter {
    #[serde(default)]
    pub permissions: Permissions,
}

/// Parse the frontmatter of a script: the leading `//` comment lines, when the first is `// ---`,
/// up to the next `// ---`. No frontmatter is the default (no permissions).
pub fn parse_frontmatter(source: &str) -> anyhow::Result<Frontmatter> {
    let mut lines = source
        .lines()
        .map(str::trim)
        .skip_while(|l| l.is_empty() || l.starts_with("#!"));
    if lines.next().map(comment_body) != Some(Some("---")) {
        return Ok(Frontmatter::default());
    }
    let mut toml_src = String::new();
    for line in lines {
        let Some(body) = comment_body(line) else {
            anyhow::bail!("frontmatter: `// ---` opened but not closed before the code starts");
        };
        if body == "---" {
            return toml::from_str(&toml_src).context("frontmatter");
        }
        toml_src.push_str(body);
        toml_src.push('\n');
    }
    anyhow::bail!("frontmatter: `// ---` opened but never closed")
}

fn comment_body(line: &str) -> Option<&str> {
    line.strip_prefix("//")
        .map(|b| b.strip_prefix(' ').unwrap_or(b).trim_end())
}

/// The part of `distri.toml` script tools read. Other tables are someone else's.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DistriToml {
    /// `[tools.<tool name>]`: what the project grants that tool without asking.
    #[serde(default)]
    pub tools: BTreeMap<String, Permissions>,
}

impl DistriToml {
    /// `Ok(default)` when the file does not exist.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("parse {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
        }
    }

    pub fn grant_for(&self, tool: &str) -> Permissions {
        self.tools.get(tool).cloned().unwrap_or_default()
    }
}

/// One remembered answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantRecord {
    /// The script the answer was given for; a different file under the same name asks again.
    pub script: PathBuf,
    /// What was asked for.
    pub permissions: Permissions,
    pub allowed: bool,
    pub decided_at: DateTime<Utc>,
}

/// `~/.distri/tool_grants.json`: project root → tool name → the answer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GrantFile {
    #[serde(default)]
    pub projects: BTreeMap<String, BTreeMap<String, GrantRecord>>,
}

/// What the store remembers for one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remembered {
    Allowed,
    Denied,
    Unknown,
}

/// The remembered answers, per project.
#[derive(Debug, Clone)]
pub struct GrantStore {
    path: PathBuf,
}

impl GrantStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn read(&self) -> anyhow::Result<GrantFile> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => {
                serde_json::from_str(&s).with_context(|| format!("parse {}", self.path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(GrantFile::default()),
            Err(e) => Err(e).with_context(|| format!("read {}", self.path.display())),
        }
    }

    /// An allow covers any request it is a superset of; a deny only the exact same request.
    pub fn lookup(
        &self,
        project: &Path,
        tool: &str,
        script: &Path,
        request: &Permissions,
    ) -> anyhow::Result<Remembered> {
        let file = self.read()?;
        let Some(rec) = file
            .projects
            .get(&project_key(project))
            .and_then(|tools| tools.get(tool))
        else {
            return Ok(Remembered::Unknown);
        };
        if rec.script != script {
            return Ok(Remembered::Unknown);
        }
        Ok(match rec.allowed {
            true if rec.permissions.covers(request) => Remembered::Allowed,
            false if rec.permissions == *request => Remembered::Denied,
            _ => Remembered::Unknown,
        })
    }

    pub fn remember(
        &self,
        project: &Path,
        tool: &str,
        script: &Path,
        request: &Permissions,
        allowed: bool,
    ) -> anyhow::Result<()> {
        let mut file = self.read()?;
        file.projects
            .entry(project_key(project))
            .or_default()
            .insert(
                tool.to_string(),
                GrantRecord {
                    script: script.to_path_buf(),
                    permissions: request.clone(),
                    allowed,
                    decided_at: Utc::now(),
                },
            );
        crate::script_tools::write_atomic(
            &self.path,
            serde_json::to_string_pretty(&file)?.as_bytes(),
        )
    }
}

fn project_key(project: &Path) -> String {
    project.to_string_lossy().into_owned()
}

/// One question: may `tool` (at `script`) have `request` in `project`?
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub project: PathBuf,
    pub tool: String,
    pub script: PathBuf,
    pub request: Permissions,
}

/// Asks the operator. `None` when there is nobody to ask (no terminal): the request is denied
/// for this call and nothing is remembered.
pub trait Approver: Send + Sync {
    fn approve(&self, req: &ApprovalRequest) -> Option<bool>;
}

/// Asks on the terminal (stderr prompt, stdin answer) when stdin is a terminal.
#[derive(Debug, Clone, Copy, Default)]
pub struct TerminalApprover;

impl Approver for TerminalApprover {
    fn approve(&self, req: &ApprovalRequest) -> Option<bool> {
        if !std::io::stdin().is_terminal() {
            return None;
        }
        let mut err = std::io::stderr();
        let _ = write!(
            err,
            "\nScript tool '{}' ({}) asks for {}.\nAllow for this project? [y/N] ",
            req.tool,
            req.script.display(),
            req.request.describe()
        );
        let _ = err.flush();
        let mut answer = String::new();
        std::io::stdin().lock().read_line(&mut answer).ok()?;
        Some(matches!(
            answer.trim().to_ascii_lowercase().as_str(),
            "y" | "yes"
        ))
    }
}

/// A fixed answer, for hosts without a terminal and for tests.
#[derive(Debug, Clone, Copy)]
pub struct FixedApprover(pub Option<bool>);

impl Approver for FixedApprover {
    fn approve(&self, _req: &ApprovalRequest) -> Option<bool> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_is_toml_in_leading_comments() {
        let src = "#!/usr/bin/env -S deno run\n// ---\n// [permissions]\n// net = [\"api.example.com\"]\n// read = [\"./data\"]\n// ---\nimport x from \"y\";\n";
        let fm = parse_frontmatter(src).unwrap();
        assert_eq!(
            fm.permissions.net,
            BTreeSet::from(["api.example.com".to_string()])
        );
        assert_eq!(fm.permissions.read, BTreeSet::from(["./data".to_string()]));
    }

    #[test]
    fn no_frontmatter_is_no_permissions() {
        assert_eq!(
            parse_frontmatter("// a comment\nexport {}").unwrap(),
            Frontmatter::default()
        );
        assert_eq!(parse_frontmatter("").unwrap(), Frontmatter::default());
    }

    #[test]
    fn env_and_run_are_not_requestable() {
        let src = "// ---\n// [permissions]\n// env = [\"HOME\"]\n// ---\n";
        let err = format!("{:#}", parse_frontmatter(src).unwrap_err());
        assert!(err.contains("unknown field `env`"), "{err}");
        assert!(parse_frontmatter("// ---\n// [permissions]\nexport {}").is_err());
    }

    #[test]
    fn grants_remember_per_project_and_script() {
        let dir = tempfile::tempdir().unwrap();
        let store = GrantStore::new(dir.path().join("grants.json"));
        let project = Path::new("/p");
        let script = Path::new("/p/.distri/tools/a.ts");
        let req = Permissions {
            net: BTreeSet::from(["x.com".into()]),
            read: BTreeSet::new(),
        };
        assert_eq!(
            store.lookup(project, "a", script, &req).unwrap(),
            Remembered::Unknown
        );
        store.remember(project, "a", script, &req, true).unwrap();
        assert_eq!(
            store.lookup(project, "a", script, &req).unwrap(),
            Remembered::Allowed
        );
        // Another project, another script, or a bigger request asks again.
        assert_eq!(
            store.lookup(Path::new("/q"), "a", script, &req).unwrap(),
            Remembered::Unknown
        );
        assert_eq!(
            store
                .lookup(project, "a", Path::new("/other.ts"), &req)
                .unwrap(),
            Remembered::Unknown
        );
        let bigger = req.union(&Permissions {
            net: BTreeSet::from(["y.com".into()]),
            read: BTreeSet::new(),
        });
        assert_eq!(
            store.lookup(project, "a", script, &bigger).unwrap(),
            Remembered::Unknown
        );
        store
            .remember(project, "a", script, &bigger, false)
            .unwrap();
        assert_eq!(
            store.lookup(project, "a", script, &bigger).unwrap(),
            Remembered::Denied
        );
    }

    #[test]
    fn distri_toml_grants_by_tool_name() {
        let t: DistriToml =
            toml::from_str("[server]\nx = 1\n[tools.lookup_order]\nnet = [\"localhost:5432\"]\n")
                .unwrap();
        assert_eq!(t.grant_for("lookup_order").net.len(), 1);
        assert!(t.grant_for("other").is_empty());
    }
}
