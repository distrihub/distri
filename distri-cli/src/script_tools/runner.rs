//! Deno as a child process, with the `@distri/tool` harness. The argv is a pure function of a
//! [`DenoRun`] so a test can pin it: `--no-prompt`, no `--allow-env`, no `--allow-run`, no
//! `--allow-ffi`, no `--allow-write`, `--allow-read` and `--allow-net` only as granted. The
//! child's environment is cleared; it gets `DENO_DIR` (our cache) and `NO_COLOR`, nothing else.
//!
//! A call's stderr is read while it runs: `__distri_ctx__ {json}` lines (`ctx.progress`,
//! `ctx.log`, `ctx.warn`) and plain lines become [`LocalToolEventKind`]s on the call's
//! [`EventSink`]; plain lines are also kept for the failure message. When the call is dropped
//! (cancelled) or times out, the child gets SIGTERM (`ctx.signal` aborts) and SIGKILL
//! [`KILL_GRACE`] later.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Context;
use distri_types::script_tool::{parse_extract_stdout, ToolExport};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use super::write_atomic;
use crate::local_events::{LocalToolEventKind, LogLevel};

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

/// How long one tool call may run, unless its `timeout` says otherwise.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// How long reading one script's `tool` export may take.
pub const EXTRACT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long `deno check` on one script may take.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(120);
/// Between SIGTERM and SIGKILL when a call is cancelled or times out.
pub const KILL_GRACE: Duration = Duration::from_secs(3);
/// How much of stderr a failed call reports.
const STDERR_TAIL: usize = 4 * 1024;
/// How much plain stderr one call keeps in memory (the oldest is dropped first).
const STDERR_KEEP: usize = 256 * 1024;

/// The prefix of a context line on stderr (`CTX_SENTINEL` in `@distri/tool`).
pub const CTX_SENTINEL: &str = "__distri_ctx__ ";

/// Where a call's progress and log lines go while it runs.
pub type EventSink = Arc<dyn Fn(LocalToolEventKind) + Send + Sync>;

/// One `__distri_ctx__` line: `{"progress": 0.62, "message": "page 5/8"}`, `{"log": "…"}`,
/// `{"warn": "…"}`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
enum CtxLine {
    Log(CtxLog),
    Warn(CtxWarn),
    Progress(CtxProgress),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct CtxLog {
    log: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct CtxWarn {
    warn: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct CtxProgress {
    progress: Option<f64>,
    #[serde(default)]
    message: Option<String>,
}

impl From<CtxLine> for LocalToolEventKind {
    fn from(line: CtxLine) -> Self {
        match line {
            CtxLine::Log(l) => LocalToolEventKind::Log {
                level: LogLevel::Log,
                line: l.log,
            },
            CtxLine::Warn(w) => LocalToolEventKind::Log {
                level: LogLevel::Warn,
                line: w.warn,
            },
            CtxLine::Progress(p) => LocalToolEventKind::Progress {
                fraction: p.progress.map(|f| f.clamp(0.0, 1.0)),
                message: p.message,
            },
        }
    }
}

/// The event a `__distri_ctx__` line carries; `None` for any other line (or a malformed one,
/// which is then a plain line).
pub fn parse_ctx_line(line: &str) -> Option<LocalToolEventKind> {
    let body = line.strip_prefix(CTX_SENTINEL)?;
    serde_json::from_str::<CtxLine>(body.trim())
        .ok()
        .map(Into::into)
}

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

    /// `deno` with the cleared environment and our `DENO_DIR`.
    fn base_command(&self, cwd: &Path) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.env_clear()
            .env("DENO_DIR", &self.deno_dir)
            .env("NO_COLOR", "1")
            .env("DENO_NO_UPDATE_CHECK", "1")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd
    }

    fn command(&self, run: &DenoRun) -> Command {
        let mut cmd = self.base_command(&run.cwd);
        cmd.args(deno_argv(run));
        cmd
    }

    /// Run `cmd` with `stdin`, streaming its stderr into `sink`. When this future is dropped
    /// or `timeout` passes, the child is terminated ([`ChildGuard`]).
    async fn spawn(
        &self,
        mut cmd: Command,
        stdin: Vec<u8>,
        timeout: Duration,
        sink: Option<EventSink>,
    ) -> anyhow::Result<RunOutcome> {
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn {}", self.binary.display()))?;
        if let Some(mut pipe) = child.stdin.take() {
            tokio::spawn(async move {
                // A script that never reads is not an error.
                let _ = pipe.write_all(&stdin).await;
                let _ = pipe.shutdown().await;
            });
        }
        let stdout = child.stdout.take().context("deno stdout")?;
        let stderr = child.stderr.take().context("deno stderr")?;
        // Its own task: it keeps draining (and emitting) after a cancel, until the child exits.
        let stderr_task = tokio::spawn(read_stderr(stderr, sink));
        let mut guard = ChildGuard(Some(child));
        let finished = tokio::time::timeout(timeout, wait_with_stdout(&mut guard, stdout)).await;
        match finished {
            Err(_) => Ok(RunOutcome::TimedOut),
            Ok(result) => {
                let (code, stdout) = result?;
                let stderr = stderr_task.await.unwrap_or_default();
                Ok(RunOutcome::Exited {
                    code,
                    stdout,
                    stderr,
                })
            }
        }
    }

    /// `deno check` on one script with the `@distri/tool` import map. `Ok(Err(lines))` carries
    /// the first error lines when it does not type-check.
    pub async fn check(&self, script: &Path, cwd: &Path) -> anyhow::Result<Result<(), String>> {
        let mut cmd = self.base_command(cwd);
        cmd.args([
            "check".to_string(),
            "--quiet".to_string(),
            format!("--import-map={}", self.import_map.display()),
            script.display().to_string(),
        ]);
        match self.spawn(cmd, Vec::new(), CHECK_TIMEOUT, None).await? {
            RunOutcome::TimedOut => Ok(Err(format!(
                "deno check timed out after {}s",
                CHECK_TIMEOUT.as_secs()
            ))),
            RunOutcome::Exited { code: Some(0), .. } => Ok(Ok(())),
            RunOutcome::Exited {
                code,
                stdout,
                stderr,
            } => {
                let text = if stderr.trim().is_empty() {
                    String::from_utf8_lossy(&stdout).into_owned()
                } else {
                    stderr
                };
                let first = first_error_lines(&text);
                Ok(Err(if first.is_empty() {
                    format!("deno check failed ({})", status_text(code))
                } else {
                    first
                }))
            }
        }
    }

    /// The run of one script through `run.ts`.
    pub fn call_run(&self, script: &Path, cwd: &Path, grant: &ResolvedGrant) -> DenoRun {
        // `run.ts <script> <cwd>`: the cwd becomes `ctx.cwd`.
        DenoRun {
            entry: self.tool_dir.join("run.ts"),
            script_arg: Some(script.to_path_buf()),
            import_map: self.import_map.clone(),
            read: grant.read.clone(),
            net: grant.net.clone(),
            cwd: cwd.to_path_buf(),
        }
    }

    /// Call a script with `input`, for at most `timeout`; its progress and log lines go to
    /// `sink` as they arrive. The output document on success.
    pub async fn call(
        &self,
        tool: &str,
        run: &DenoRun,
        input: &serde_json::Value,
        timeout: Duration,
        sink: Option<EventSink>,
    ) -> Result<CallOutput, ScriptToolError> {
        let stdin = serde_json::to_vec(input).map_err(|e| ScriptToolError::Io(e.into()))?;
        match self.spawn(self.command(run), stdin, timeout, sink).await? {
            RunOutcome::TimedOut => Err(ScriptToolError::TimedOut {
                tool: tool.to_string(),
                secs: timeout.as_secs(),
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
    pub async fn extract(
        &self,
        script: &Path,
        cwd: &Path,
    ) -> Result<Option<ToolExport>, ExtractFailure> {
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
        })
        .map_err(|e| ExtractFailure::Io(e.into()))?;
        match self
            .spawn(self.command(&run), stdin, EXTRACT_TIMEOUT, None)
            .await?
        {
            RunOutcome::TimedOut => Err(ExtractFailure::TimedOut(EXTRACT_TIMEOUT)),
            RunOutcome::Exited {
                code: Some(0),
                stdout,
                ..
            } => parse_extract_stdout(&stdout).map_err(|e| ExtractFailure::Script(e.to_string())),
            RunOutcome::Exited { code, stderr, .. } => {
                let tail = tail(&stderr);
                Err(ExtractFailure::Script(match code {
                    Some(c) if tail.is_empty() => format!("import failed (exit {c})"),
                    _ => format!("import failed: {tail}"),
                }))
            }
        }
    }
}

/// Why reading a script's `tool` export failed.
#[derive(Debug, thiserror::Error)]
pub enum ExtractFailure {
    /// The script itself is broken (an import-time throw, a bad export). The same source fails
    /// the same way, so this is cached by content hash.
    #[error("{0}")]
    Script(String),
    /// It did not finish in time; may pass on the next scan, so it is not cached.
    #[error("import timed out after {}s", .0.as_secs())]
    TimedOut(Duration),
    /// Deno could not be run; not the script's fault, not cached.
    #[error(transparent)]
    Io(#[from] anyhow::Error),
}

/// Wait for the child's exit while reading all of its stdout.
async fn wait_with_stdout(
    guard: &mut ChildGuard,
    mut stdout: impl AsyncRead + Unpin,
) -> anyhow::Result<(Option<i32>, Vec<u8>)> {
    let child = guard.0.as_mut().context("child already reaped")?;
    let mut out = Vec::new();
    let (status, read) = tokio::join!(child.wait(), stdout.read_to_end(&mut out));
    let status = status.context("wait for deno")?;
    read.context("read deno stdout")?;
    Ok((status.code(), out))
}

/// Read stderr line by line: context lines go to `sink`, plain lines go to `sink` as logs and
/// into the returned text (the failure tail never shows a context line).
async fn read_stderr(stderr: impl AsyncRead + Unpin, sink: Option<EventSink>) -> String {
    let mut reader = BufReader::new(stderr);
    let mut plain = String::new();
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&buf);
        let line = text.trim_end_matches(['\n', '\r']);
        if let Some(kind) = parse_ctx_line(line) {
            if let Some(sink) = &sink {
                sink(kind);
            }
            continue;
        }
        plain.push_str(line);
        plain.push('\n');
        if plain.len() > STDERR_KEEP {
            let mut cut = plain.len() - STDERR_KEEP;
            while !plain.is_char_boundary(cut) {
                cut += 1;
            }
            plain.drain(..cut);
        }
        if let (Some(sink), false) = (&sink, line.trim().is_empty()) {
            sink(LocalToolEventKind::Log {
                level: LogLevel::Log,
                line: line.to_string(),
            });
        }
    }
    plain
}

/// Owns a running child. Dropped before the child exited (the call was cancelled or timed
/// out), it sends SIGTERM and SIGKILL [`KILL_GRACE`] later; `kill_on_drop` alone would be an
/// immediate SIGKILL, and the script's `ctx.signal` would never fire.
struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let Some(mut child) = self.0.take() else {
            return;
        };
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        terminate(child);
    }
}

#[cfg(unix)]
fn terminate(mut child: Child) {
    if let (Some(pid), Ok(runtime)) = (child.id(), tokio::runtime::Handle::try_current()) {
        // SAFETY: kill(2) on our own child, which has not been reaped (try_wait said it is
        // still running), so the pid cannot have been reused.
        let sent = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } == 0;
        if sent {
            runtime.spawn(async move {
                if tokio::time::timeout(KILL_GRACE, child.wait())
                    .await
                    .is_err()
                {
                    let _ = child.kill().await;
                }
            });
            return;
        }
    }
    let _ = child.start_kill();
}

#[cfg(not(unix))]
fn terminate(mut child: Child) {
    let _ = child.start_kill();
}

/// The first error of a `deno check` run: its lines up to the first blank one (at most 8).
fn first_error_lines(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| !l.trim().is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("\n")
}

fn status_text(code: Option<i32>) -> String {
    match code {
        Some(c) => format!("exit {c}"),
        None => "killed by a signal".to_string(),
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
    /// The script, passed to `run.ts` as its argument (followed by `cwd`, for `ctx.cwd`).
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
        argv.push(run.cwd.display().to_string());
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
    #[error("script tool '{tool}': input does not match its schema: {detail}")]
    InvalidInput { tool: String, detail: String },
    #[error("script tool '{tool}' has an invalid input schema (`parameters`): {detail}")]
    BadSchema { tool: String, detail: String },
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
        status: status_text(code),
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
                "/p",
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
    fn ctx_lines_are_typed_events_and_other_lines_are_not() {
        assert_eq!(
            parse_ctx_line(r#"__distri_ctx__ {"progress":0.62,"message":"page 5/8"}"#),
            Some(LocalToolEventKind::Progress {
                fraction: Some(0.62),
                message: Some("page 5/8".into())
            })
        );
        assert_eq!(
            parse_ctx_line(r#"__distri_ctx__ {"progress":null}"#),
            Some(LocalToolEventKind::Progress {
                fraction: None,
                message: None
            })
        );
        assert_eq!(
            parse_ctx_line(r#"__distri_ctx__ {"progress":7}"#),
            Some(LocalToolEventKind::Progress {
                fraction: Some(1.0),
                message: None
            })
        );
        assert_eq!(
            parse_ctx_line(r#"__distri_ctx__ {"log":"text"}"#),
            Some(LocalToolEventKind::Log {
                level: LogLevel::Log,
                line: "text".into()
            })
        );
        assert_eq!(
            parse_ctx_line(r#"__distri_ctx__ {"warn":"careful"}"#),
            Some(LocalToolEventKind::Log {
                level: LogLevel::Warn,
                line: "careful".into()
            })
        );
        assert_eq!(parse_ctx_line("plain line"), None);
        assert_eq!(parse_ctx_line("__distri_ctx__ not json"), None);
        assert_eq!(
            parse_ctx_line(r#"__distri_ctx__ {"log":"a","extra":1}"#),
            None
        );
    }

    #[tokio::test]
    async fn stderr_framed_lines_never_reach_the_tail() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink: EventSink = {
            let seen = seen.clone();
            Arc::new(move |k| seen.lock().unwrap().push(k))
        };
        let input: &[u8] =
            b"one\n__distri_ctx__ {\"warn\":\"w\"}\n\ntwo\r\n__distri_ctx__ {\"progress\":0.5}";
        let plain = read_stderr(input, Some(sink)).await;
        assert_eq!(plain, "one\n\ntwo\n");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 4, "{seen:?}");
        assert!(
            matches!(&seen[0], LocalToolEventKind::Log { level: LogLevel::Log, line } if line == "one")
        );
        assert!(
            matches!(&seen[1], LocalToolEventKind::Log { level: LogLevel::Warn, line } if line == "w")
        );
        assert!(
            matches!(&seen[3], LocalToolEventKind::Progress { fraction: Some(f), .. } if *f == 0.5)
        );
    }

    #[test]
    fn deno_check_reports_its_first_error() {
        let out = "\nerror: TS2322 [ERROR]: Type 'number' is not assignable to type 'string'.\n  return 1;\n    at file:///a.ts:3:3\n\nTS2345 [ERROR]: other\n";
        assert_eq!(
            first_error_lines(out),
            "error: TS2322 [ERROR]: Type 'number' is not assignable to type 'string'.\n  return 1;\n    at file:///a.ts:3:3"
        );
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
