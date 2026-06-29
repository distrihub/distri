// Platform integration tests must run sequentially (--test-threads=1) because
// concurrent SSE streams exhaust actix-web worker threads. See backlog.md.
use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// Global lock to serialize platform tests that stream SSE from the server.
/// Without this, concurrent streams exhaust server worker threads.
static STREAM_LOCK: Mutex<()> = Mutex::new(());

fn base_url() -> Option<String> {
    env::var("DISTRI_BASE_URL").ok()
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn run_cli(base_url: &str, args: &[&str]) -> Result<()> {
    let output = Command::new("distri")
        .arg("--base-url")
        .arg(base_url)
        .args(args)
        .output()
        .with_context(|| format!("running distri with args {:?}", args))?;

    if !output.status.success() {
        anyhow::bail!(
            "distri command {:?} failed (status={})\nstdout:\n{}\nstderr:\n{}",
            args,
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

fn ensure_base_url() -> Result<Option<String>> {
    let url = base_url();
    if url.is_none() {
        eprintln!("Skipping smoke test; set DISTRI_BASE_URL to enable.");
    }
    Ok(url)
}

fn write_agent_fixture(dest: &Path, source_name: &str) -> Result<()> {
    let source = repo_root().join("agents").join(source_name);
    let content = fs::read_to_string(&source)
        .with_context(|| format!("reading agent fixture {}", source.display()))?;
    fs::write(dest, content).with_context(|| format!("writing {}", dest.display()))?;
    Ok(())
}

#[test]
fn smoke_agents_list() -> Result<()> {
    let Some(base_url) = ensure_base_url()? else {
        return Ok(());
    };
    run_cli(&base_url, &["agents", "list"])
}

#[test]
fn smoke_agents_push_single() -> Result<()> {
    let Some(base_url) = ensure_base_url()? else {
        return Ok(());
    };
    let agent_path = repo_root().join("agents").join("cli_agent.md");
    run_cli(
        &base_url,
        &[
            "agents",
            "push",
            agent_path
                .to_str()
                .context("agent path is not valid UTF-8")?,
        ],
    )
}

#[test]
fn smoke_agents_push_all() -> Result<()> {
    let Some(base_url) = ensure_base_url()? else {
        return Ok(());
    };
    let dir = tempfile::tempdir().context("creating temp dir")?;
    let first = dir.path().join("agent_one.md");
    let second = dir.path().join("agent_two.md");
    write_agent_fixture(&first, "distri.md")?;
    write_agent_fixture(&second, "cli_agent.md")?;
    run_cli(
        &base_url,
        &[
            "agents",
            "push",
            dir.path()
                .to_str()
                .context("temp dir path is not valid UTF-8")?,
            "--all",
        ],
    )
}

#[test]
fn smoke_tools_list() -> Result<()> {
    let Some(base_url) = ensure_base_url()? else {
        return Ok(());
    };
    run_cli(&base_url, &["tools", "list"])
}

#[test]
fn smoke_tools_invoke() -> Result<()> {
    let Some(base_url) = ensure_base_url()? else {
        return Ok(());
    };
    let tool_name = match env::var("DISTRI_SMOKE_TOOL_NAME") {
        Ok(name) => name,
        Err(_) => {
            eprintln!("Skipping tools invoke smoke test; set DISTRI_SMOKE_TOOL_NAME.");
            return Ok(());
        }
    };
    let tool_input = env::var("DISTRI_SMOKE_TOOL_INPUT").unwrap_or_else(|_| "{}".to_string());
    run_cli(
        &base_url,
        &["tools", "invoke", &tool_name, "--input", &tool_input],
    )
}

#[test]
fn smoke_run_browser_agent_task() -> Result<()> {
    let Some(base_url) = ensure_base_url()? else {
        return Ok(());
    };
    run_cli(
        &base_url,
        &[
            "run",
            "--task",
            "Find all open job positions in google singapore",
        ],
    )
}

// ============================================================
// CLI subcommand smoke tests
// ============================================================

#[test]
fn smoke_connections_list() -> Result<()> {
    let Some(base_url) = ensure_base_url()? else {
        return Ok(());
    };
    run_cli(&base_url, &["connections", "list"])
}

#[test]
fn smoke_secrets_list() -> Result<()> {
    let Some(base_url) = ensure_base_url()? else {
        return Ok(());
    };
    run_cli(&base_url, &["secrets", "list"])
}

#[test]
fn smoke_threads_list() -> Result<()> {
    let Some(base_url) = ensure_base_url()? else {
        return Ok(());
    };
    run_cli(&base_url, &["threads", "list"])
}

// ============================================================
// Platform tool integration tests (run agent with --task)
// ============================================================

fn run_cli_capture(base_url: &str, args: &[&str]) -> Result<String> {
    let output = Command::new("distri")
        .arg("--base-url")
        .arg(base_url)
        .args(args)
        .output()
        .with_context(|| format!("running distri with args {:?}", args))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        anyhow::bail!(
            "distri command {:?} failed (status={})\nstdout:\n{}\nstderr:\n{}",
            args,
            output.status,
            stdout,
            stderr
        );
    }

    Ok(format!("{}{}", stdout, stderr))
}

#[test]
fn smoke_platform_list_connections() -> Result<()> {
    let _lock = STREAM_LOCK.lock().unwrap();
    let Some(base_url) = ensure_base_url()? else {
        return Ok(());
    };
    let output = run_cli_capture(
        &base_url,
        &[
            "run",
            "--task",
            "how many connections do i have? just give me the count",
        ],
    )?;
    // Agent should use distri_platform list_connections and respond
    assert!(
        output.contains("distri_platform")
            || output.contains("connection")
            || output.contains("Connection"),
        "expected platform tool usage or connection mention in output: {}",
        &output[..output.len().min(500)]
    );
    Ok(())
}

#[test]
fn smoke_platform_list_agents() -> Result<()> {
    let _lock = STREAM_LOCK.lock().unwrap();
    let Some(base_url) = ensure_base_url()? else {
        return Ok(());
    };
    let output = run_cli_capture(
        &base_url,
        &["run", "--task", "list all my agents, just the names"],
    )?;
    // The distri agent should always exist
    assert!(
        output.contains("distri"),
        "expected 'distri' agent in output: {}",
        &output[..output.len().min(500)]
    );
    Ok(())
}

#[test]
fn smoke_platform_list_secrets() -> Result<()> {
    let _lock = STREAM_LOCK.lock().unwrap();
    let Some(base_url) = ensure_base_url()? else {
        return Ok(());
    };
    let output = run_cli_capture(
        &base_url,
        &["run", "--task", "list my secrets, just key names"],
    )?;
    // Should use distri_platform list_secrets
    assert!(
        output.contains("distri_platform")
            || output.contains("secret")
            || output.contains("Secret")
            || output.contains("No secrets"),
        "expected secrets-related output: {}",
        &output[..output.len().min(500)]
    );
    Ok(())
}

#[test]
fn smoke_platform_list_skills() -> Result<()> {
    let _lock = STREAM_LOCK.lock().unwrap();
    let Some(base_url) = ensure_base_url()? else {
        return Ok(());
    };
    let output = run_cli_capture(
        &base_url,
        &["run", "--task", "how many skills do i have? just the count"],
    )?;
    assert!(
        output.contains("distri_platform") || output.contains("skill") || output.contains("Skill"),
        "expected skills-related output: {}",
        &output[..output.len().min(500)]
    );
    Ok(())
}
