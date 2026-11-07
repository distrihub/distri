use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use distri::ExternalToolRegistry;
use distri_cli::interact::SharedBroker;
use distri_cli::local_events::{LocalEvents, LocalToolEventKind, LogLevel};
use distri_cli::local_tools;
pub use distri_cli::local_tools::{register_all, register_approval_handler};
use distri_cli::script_tools::{self, Discovery, EventSink, ScriptToolHost, ScriptToolsConfig};
use distri_types::ToolDefinition;

use crate::{COLOR_BRIGHT_GREEN, COLOR_BRIGHT_MAGENTA, COLOR_BRIGHT_YELLOW, COLOR_RESET};

/// The names the built-in handlers own; a script tool cannot take one.
pub fn builtin_tool_names() -> BTreeSet<String> {
    local_tools::tool_definitions()
        .into_iter()
        .map(|d| d.name)
        .chain([
            "execute_command".to_string(),
            "approval_request".to_string(),
        ])
        .collect()
}

/// The script-tool host for a project; permission prompts go to `broker`.
pub fn script_tool_host(
    workspace_root: &Path,
    distri_toml: Option<PathBuf>,
    broker: SharedBroker,
) -> anyhow::Result<Arc<ScriptToolHost>> {
    let cfg = ScriptToolsConfig::for_project(workspace_root, distri_toml)?;
    Ok(Arc::new(ScriptToolHost::new(cfg, broker)?))
}

/// Print what a scan could not register (once per invocation): the Deno-missing notice and
/// every script that failed to load. A failure the extract cache already knew (the script has
/// not changed since) is one line, not the whole stack again.
pub fn print_discovery_problems(discovery: &Discovery) {
    if let Some(notice) = discovery.deno_missing_notice() {
        eprintln!("{}{}{}", COLOR_BRIGHT_YELLOW, notice, COLOR_RESET);
    }
    for p in &discovery.problems {
        if p.cached {
            eprintln!(
                "{}Script tool {} not registered: {} (unchanged since it failed; `distri tools check` for details){}",
                COLOR_BRIGHT_YELLOW,
                p.path.display(),
                p.summary(),
                COLOR_RESET
            );
        } else {
            eprintln!(
                "{}Script tool {} not registered: {}{}",
                COLOR_BRIGHT_YELLOW,
                p.path.display(),
                p.message,
                COLOR_RESET
            );
        }
    }
}

/// Discover `.distri/tools/*.ts` and `~/.distri/tools/*.ts`, register a handler for each next
/// to the built-ins, and return their definitions. Permission prompts go to `broker`; progress
/// and log lines of each call go out on `events`. Problems are printed, never fatal: the
/// built-ins keep working.
pub async fn register_script_tools(
    registry: &ExternalToolRegistry,
    workspace_root: &Path,
    distri_toml: Option<PathBuf>,
    broker: SharedBroker,
    events: LocalEvents,
) -> Vec<ToolDefinition> {
    let host = match script_tool_host(workspace_root, distri_toml, broker) {
        Ok(h) => h,
        Err(e) => {
            eprintln!(
                "{}Script tools disabled: {:#}{}",
                COLOR_BRIGHT_YELLOW, e, COLOR_RESET
            );
            return Vec::new();
        }
    };
    let discovery = host.discover(&builtin_tool_names()).await;
    print_discovery_problems(&discovery);
    script_tools::register(&host, registry, &discovery, events)
}

/// Prints a script tool's `ctx.progress` / `ctx.log` / `ctx.warn` and stderr lines on stderr
/// as they arrive (`distri tools test`).
pub fn print_script_events() -> EventSink {
    Arc::new(|kind| match kind {
        LocalToolEventKind::Progress { fraction, message } => {
            let pct = fraction
                .map(|f| format!("{:>3.0}%", f * 100.0))
                .unwrap_or_else(|| "   …".to_string());
            eprintln!(
                "{}[{}]{} {}",
                COLOR_BRIGHT_MAGENTA,
                pct,
                COLOR_RESET,
                message.unwrap_or_default()
            );
        }
        LocalToolEventKind::Log {
            level: LogLevel::Log,
            line,
        } => eprintln!("{line}"),
        LocalToolEventKind::Log {
            level: LogLevel::Warn,
            line,
        } => eprintln!("{}warning:{} {}", COLOR_BRIGHT_YELLOW, COLOR_RESET, line),
        // `tools test` prints the result itself.
        LocalToolEventKind::Finished(_) => {}
    })
}

/// `distri tools check [name]`: `deno check` and the schema of each script tool. Prints one
/// line per tool and returns `false` when any failed.
pub async fn check_script_tools(host: &ScriptToolHost, name: Option<&str>) -> bool {
    let discovery = host.discover(&builtin_tool_names()).await;
    if let Some(notice) = discovery.deno_missing_notice() {
        eprintln!("{}{}{}", COLOR_BRIGHT_YELLOW, notice, COLOR_RESET);
        return false;
    }
    let reports = host.check(&discovery, name).await;
    if reports.is_empty() {
        match name {
            Some(n) => eprintln!(
                "no script tool '{}' in {} or ~/.distri/tools",
                n,
                host.config().project_tools_dir().display()
            ),
            None => eprintln!(
                "no script tools in {} or ~/.distri/tools",
                host.config().project_tools_dir().display()
            ),
        }
        return name.is_none();
    }
    let mut ok = true;
    for r in &reports {
        match &r.result {
            Ok(()) => println!(
                "{}✓{} {}  deno check · schema ok",
                COLOR_BRIGHT_GREEN, COLOR_RESET, r.name
            ),
            Err(lines) => {
                ok = false;
                let mut lines = lines.lines();
                println!(
                    "{}✗{} {}  {}",
                    COLOR_BRIGHT_YELLOW,
                    COLOR_RESET,
                    r.name,
                    lines.next().unwrap_or_default()
                );
                for l in lines {
                    println!("    {l}");
                }
            }
        }
    }
    ok
}
