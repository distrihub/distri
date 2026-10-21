//! Deno as a child process, with the `@distri/tool` harness. The argv is a pure function of a
//! [`DenoRun`] so a test can pin it: `--no-prompt`, no `--allow-env`, no `--allow-run`, no
//! `--allow-ffi`, no `--allow-write`, `--allow-read` and `--allow-net` only as granted. The
//! child's environment is cleared; it gets `DENO_DIR` (our cache) and `NO_COLOR`, nothing else.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Context;
use distri_types::script_tool::{parse_extract_stdout, ToolExport};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::write_atomic;

/// The `@distri/tool` files, embedded from `distri-tool/` and written to `<run_dir>/tool/`.
const TOOL_FILES: &[(&str, &str)] = &[
    (
        "mod.ts",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../distri-tool/mod.ts"
        )),
    ),
    (
        "run.ts",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../distri-tool/run.ts"
        )),
    ),
    (
        "extract.ts",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../distri-tool/extract.ts"
        )),
    ),
];

/// The specifiers that resolve to the embedded `@distri/tool`.
const TOOL_SPECIFIERS: &[&str] = &["jsr:@distri/tool", "@distri/tool"];

/// How long one tool call may run.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// How long reading one script's `tool` export may take.
pub const EXTRACT_TIMEOUT: Duration = Duration::from_secs(30);
/// How much of stderr a failed call reports.
const STDERR_TAIL: usize = 4 * 1024;

/// Where the binary is and where the harness lives.
#[derive(Debug, Clone)]
pub struct Deno {
    pub binary: PathBuf,
    /// `<run_dir>/tool/`: the materialised `@distri/tool`.
    pub tool_dir: PathBuf,
    /// The import map that points `jsr:@distri/tool` at `tool_dir`.
    pub import_map: PathBuf,
    /// `DENO_DIR` for every child: the module cache, ours, never the user's.
    pub deno_dir: PathBuf,
}

/// `{"imports": {...}}`.
#[derive(Debug, Serialize)]
struct ImportMap {
    imports: BTreeMap<String, String>,
}

/// The extractor's input document.
#[derive(Debug, Serialize, Deserialize)]
struct ExtractInput {
    path: PathBuf,
}

/// Find `deno`: `DISTRI_DENO` when set, else `deno` on `PATH`. `None` when there is none.
pub fn locate_deno() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("DISTRI_DENO").filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    which::which("deno").ok()
}

impl Deno {
    /// Write the harness and the import map under `run_dir` (idempotent) for `binary`.
    pub fn prepare(binary: PathBuf, run_dir: &Path, deno_dir: PathBuf) -> anyhow::Result<Self> {
        let tool_dir = run_dir.join("tool");
        std::fs::create_dir_all(&tool_dir)
            .with_context(|| format!("create {}", tool_dir.display()))?;
        for (name, content) in TOOL_FILES {
            let path = tool_dir.join(name);
            if std::fs::read_to_string(&path).ok().as_deref() != Some(*content) {
                write_atomic(&path, content.as_bytes())?;
            }
        }
        let tool_dir = tool_dir.canonicalize()?;
        let mod_url = format!("file://{}", tool_dir.join("mod.ts").display());
        let map = ImportMap {
            imports: TOOL_SPECIFIERS
                .iter()
                .map(|s| (s.to_string(), mod_url.clone()))
                .collect(),
        };
        let import_map = run_dir.join("import_map.json");
        let body = serde_json::to_vec_pretty(&map)?;
        if std::fs::read(&import_map).ok().as_deref() != Some(body.as_slice()) {
            write_atomic(&import_map, &body)?;
        }
        std::fs::create_dir_all(&deno_dir)
            .with_context(|| format!("create {}", deno_dir.display()))?;
        Ok(Self {
            binary,
            tool_dir,
            import_map,
            deno_dir,
        })
    }

    fn command(&self, run: &DenoRun) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.env_clear()
            .env("DENO_DIR", &self.deno_dir)
            .env("NO_COLOR", "1")
            .env("DENO_NO_UPDATE_CHECK", "1")
            .args(deno_argv(run))
            .current_dir(&run.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd
    }

    async fn spawn(
        &self,
        run: &DenoRun,
        stdin: Vec<u8>,
        timeout: Duration,
    ) -> anyhow::Result<RunOutcome> {
        let mut child = self
            .command(run)
            .spawn()
            .with_context(|| format!("spawn {}", self.binary.display()))?;
        if let Some(mut pipe) = child.stdin.take() {
            tokio::spawn(async move {
                // A script that never reads is not an error.
                let _ = pipe.write_all(&stdin).await;
                let _ = pipe.shutdown().await;
            });
        }
        // Dropping the child on timeout kills it (`kill_on_drop`).
        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Err(_) => Ok(RunOutcome::TimedOut),
            Ok(out) => {
                let out = out.context("wait for deno")?;
                Ok(RunOutcome::Exited {
                    code: out.status.code(),
                    stdout: out.stdout,
                    stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                })
            }
        }
    }

    /// The run of one script through `run.ts`.
    pub fn call_run(&self, script: &Path, cwd: &Path, grant: &ResolvedGrant) -> DenoRun {
        DenoRun {
            entry: self.tool_dir.join("run.ts"),
            script_arg: Some(script.to_path_buf()),
            import_map: self.import_map.clone(),
            read: grant.read.clone(),
            net: grant.net.clone(),
            cwd: cwd.to_path_buf(),
        }
    }

    /// Call a script with `input`. The output document on success.
    pub async fn call(
        &self,
        tool: &str,
        run: &DenoRun,
        input: &serde_json::Value,
    ) -> Result<CallOutput, ScriptToolError> {
        let stdin = serde_json::to_vec(input).map_err(|e| ScriptToolError::Io(e.into()))?;
        match self.spawn(run, stdin, CALL_TIMEOUT).await? {
            RunOutcome::TimedOut => Err(ScriptToolError::TimedOut {
                tool: tool.to_string(),
                secs: CALL_TIMEOUT.as_secs(),
            }),
            RunOutcome::Exited {
                code: Some(0),
                stdout,
                stderr,
            } => {
                let text = String::from_utf8_lossy(&stdout);
                let value =
                    serde_json::from_str(text.trim()).map_err(|e| ScriptToolError::BadOutput {
                        tool: tool.to_string(),
                        detail: e.to_string(),
                    })?;
                Ok(CallOutput { value, stderr })
            }
            RunOutcome::Exited { code, stderr, .. } => Err(classify_failure(tool, code, &stderr)),
        }
    }

    /// Read a script's `tool` export with `extract.ts`, under read access to its own folder and
    /// nothing else. `Ok(None)` when the script exports no tool.
    pub async fn extract(&self, script: &Path, cwd: &Path) -> anyhow::Result<Option<ToolExport>> {
        let folder = script
            .parent()
            .context("script has no parent folder")?
            .to_path_buf();
        let run = DenoRun {
            entry: self.tool_dir.join("extract.ts"),
            script_arg: None,
            import_map: self.import_map.clone(),
            read: vec![folder],
            net: Vec::new(),
            cwd: cwd.to_path_buf(),
        };
        let stdin = serde_json::to_vec(&ExtractInput {
            path: script.to_path_buf(),
        })?;
        match self.spawn(&run, stdin, EXTRACT_TIMEOUT).await? {
            RunOutcome::TimedOut => {
                anyhow::bail!("import timed out after {}s", EXTRACT_TIMEOUT.as_secs())
            }
            RunOutcome::Exited {
                code: Some(0),
                stdout,
                ..
            } => Ok(parse_extract_stdout(&stdout)?),
            RunOutcome::Exited { code, stderr, .. } => {
                let tail = tail(&stderr);
                match code {
                    Some(c) if tail.is_empty() => anyhow::bail!("import failed (exit {c})"),
                    _ => anyhow::bail!("import failed: {tail}"),
                }
            }
        }
    }
}

/// What a successful call produced.
#[derive(Debug, Clone)]
pub struct CallOutput {
    /// The stdout document.
    pub value: serde_json::Value,
    /// The script's logs.
    pub stderr: String,
}

enum RunOutcome {
    Exited {
        code: Option<i32>,
        stdout: Vec<u8>,
        stderr: String,
    },
    TimedOut,
}

/// The read paths and net hosts one run gets, resolved to what Deno takes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedGrant {
    pub read: Vec<PathBuf>,
    pub net: Vec<String>,
}

/// Everything the argv depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenoRun {
    /// `run.ts` or `extract.ts`.
    pub entry: PathBuf,
    /// The script, passed to `run.ts` as its argument.
    pub script_arg: Option<PathBuf>,
    pub import_map: PathBuf,
    pub read: Vec<PathBuf>,
    pub net: Vec<String>,
    pub cwd: PathBuf,
}

/// The exact argv after the binary. Never `-A`, `--allow-env`, `--allow-run`, `--allow-ffi`,
/// `--allow-sys` or `--allow-write`.
pub fn deno_argv(run: &DenoRun) -> Vec<String> {
    let mut argv = vec![
        "run".to_string(),
        "--no-prompt".to_string(),
        "--quiet".to_string(),
        format!("--import-map={}", run.import_map.display()),
    ];
    if !run.read.is_empty() {
        let read: Vec<String> = run.read.iter().map(|p| p.display().to_string()).collect();
        argv.push(format!("--allow-read={}", read.join(",")));
    }
    if !run.net.is_empty() {
        argv.push(format!("--allow-net={}", run.net.join(",")));
    }
    argv.push(run.entry.display().to_string());
    if let Some(script) = &run.script_arg {
        argv.push(script.display().to_string());
    }
    argv
}

/// Why a call failed. The `Display` is what the agent sees.
#[derive(Debug, thiserror::Error)]
pub enum ScriptToolError {
    #[error(
        "NotCapable: script tool '{tool}' needs {permission} access to {target}, which is outside its grant. {hint}"
    )]
    NotCapable {
        tool: String,
        /// `read`, `net`, `env`, `run`, `write`, …
        permission: String,
        target: String,
        hint: String,
    },
    #[error("script tool '{tool}' failed ({status}): {stderr}")]
    Failed {
        tool: String,
        status: String,
        stderr: String,
    },
    #[error("script tool '{tool}' timed out after {secs}s")]
    TimedOut { tool: String, secs: u64 },
    #[error(
        "script tool '{tool}' did not print one JSON document on stdout ({detail}); log with console.error, which goes to stderr"
    )]
    BadOutput { tool: String, detail: String },
    #[error(transparent)]
    Io(#[from] anyhow::Error),
}

fn permission_error() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Deno 2 throws `NotCapable`, Deno 1 `PermissionDenied`; the message is the same.
        Regex::new(r"(?:NotCapable|PermissionDenied): Requires (\w+) access to (.+?), (?:run again|specify)")
            .expect("valid regex")
    })
}

fn classify_failure(tool: &str, code: Option<i32>, stderr: &str) -> ScriptToolError {
    if let Some(c) = permission_error().captures(stderr) {
        let permission = c[1].to_string();
        let hint = match permission.as_str() {
            "read" | "net" => format!(
                "Request it in the script's frontmatter ([permissions] {permission} = [...]) or grant it under [tools.{tool}] in distri.toml."
            ),
            other => format!("Script tools are never granted {other} access."),
        };
        return ScriptToolError::NotCapable {
            tool: tool.to_string(),
            permission,
            target: c[2].to_string(),
            hint,
        };
    }
    ScriptToolError::Failed {
        tool: tool.to_string(),
        status: match code {
            Some(c) => format!("exit {c}"),
            None => "killed by a signal".to_string(),
        },
        stderr: tail(stderr),
    }
}

fn tail(s: &str) -> String {
    let s = s.trim();
    let mut start = s.len().saturating_sub(STDERR_TAIL);
    while !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_is_pinned_and_never_grants_env_or_run() {
        let run = DenoRun {
            entry: "/h/.distri/run/tool/run.ts".into(),
            script_arg: Some("/p/.distri/tools/a.ts".into()),
            import_map: "/h/.distri/run/import_map.json".into(),
            read: vec!["/p/.distri/tools".into(), "/p/data".into()],
            net: vec!["localhost:5432".into()],
            cwd: "/p".into(),
        };
        let argv = deno_argv(&run);
        assert_eq!(
            argv,
            vec![
                "run",
                "--no-prompt",
                "--quiet",
                "--import-map=/h/.distri/run/import_map.json",
                "--allow-read=/p/.distri/tools,/p/data",
                "--allow-net=localhost:5432",
                "/h/.distri/run/tool/run.ts",
                "/p/.distri/tools/a.ts",
            ]
        );
        assert!(!argv.iter().any(|a| a == "-A"
            || a.starts_with("--allow-env")
            || a.starts_with("--allow-run")
            || a.starts_with("--allow-ffi")
            || a.starts_with("--allow-sys")
            || a.starts_with("--allow-write")
            || a.starts_with("--allow-all")));
    }

    #[test]
    fn permission_errors_become_not_capable_for_both_deno_majors() {
        for stderr in [
            "error: Uncaught (in promise) PermissionDenied: Requires read access to \"/etc/hosts\", run again with the --allow-read flag\n    at ...",
            "NotCapable: Requires read access to \"/etc/hosts\", run again with the --allow-read flag",
        ] {
            let e = classify_failure("lookup_order", Some(1), stderr);
            let msg = e.to_string();
            assert!(matches!(e, ScriptToolError::NotCapable { ref permission, .. } if permission == "read"));
            assert!(msg.starts_with("NotCapable: script tool 'lookup_order' needs read access to \"/etc/hosts\""), "{msg}");
        }
        let env = classify_failure(
            "t",
            Some(1),
            "NotCapable: Requires env access to \"HOME\", run again with the --allow-env flag",
        );
        assert!(env.to_string().contains("never granted env access"));
        assert!(matches!(
            classify_failure("t", Some(2), "Error: boom"),
            ScriptToolError::Failed { .. }
        ));
    }
}
