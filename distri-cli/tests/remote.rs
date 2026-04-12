/// Integration tests for remote execution mode (`--remote` flag).
///
/// These tests require a running distri-cloud server with:
///   - `SANDBOX_ENABLED=true`
///   - browsr router + orchestrator running
///   - Container image with `distri` binary baked in
///
/// All tests are `#[ignore]` — run them explicitly with:
///   cargo test -p distri-cli --test remote -- --ignored --test-threads=1
///
/// Required env vars:
///   DISTRI_BASE_URL  — e.g. http://localhost:1341/v1
///   DISTRI_API_KEY   — valid API key for the server
use anyhow::{Context, Result};
use std::env;
use std::process::Command;
use std::sync::Mutex;

/// Serialize tests — concurrent SSE streams exhaust server worker threads.
static STREAM_LOCK: Mutex<()> = Mutex::new(());

fn base_url() -> Option<String> {
    env::var("DISTRI_BASE_URL").ok()
}

fn run_cli(base_url: &str, args: &[&str]) -> Result<String> {
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
            "distri {:?} failed (status={})\nstdout:\n{}\nstderr:\n{}",
            args,
            output.status,
            stdout,
            stderr,
        );
    }

    Ok(stdout)
}

// ── Smoke tests ────────────────────────────────────────────────────────────

/// Minimal remote run — verifies the remote execution path works end-to-end.
///
/// Expected: exits in ~2-5 seconds with exit code 0.
#[test]
#[ignore]
fn remote_smoke_say_hello() -> Result<()> {
    let _lock = STREAM_LOCK.lock().unwrap();
    let Some(base_url) = base_url() else {
        eprintln!("Skipping; set DISTRI_BASE_URL");
        return Ok(());
    };
    run_cli(
        &base_url,
        &[
            "run",
            "--agent",
            "distri_runner",
            "--task",
            "say hello",
            "--remote",
        ],
    )?;
    Ok(())
}

/// Remote run with `--overrides` (equivalent to `--remote`).
#[test]
#[ignore]
fn remote_smoke_overrides_flag() -> Result<()> {
    let _lock = STREAM_LOCK.lock().unwrap();
    let Some(base_url) = base_url() else {
        eprintln!("Skipping; set DISTRI_BASE_URL");
        return Ok(());
    };
    run_cli(
        &base_url,
        &[
            "run",
            "--agent",
            "distri_runner",
            "--task",
            "say hello",
            "--overrides",
            r#"{"remote":true}"#,
        ],
    )?;
    Ok(())
}

// ── Functional tests ───────────────────────────────────────────────────────

/// Remote run that exercises Python + file I/O in the container.
///
/// Verifies the container has python3 and the agent can write + execute a script.
#[test]
#[ignore]
fn remote_python_pandas_task() -> Result<()> {
    let _lock = STREAM_LOCK.lock().unwrap();
    let Some(base_url) = base_url() else {
        eprintln!("Skipping; set DISTRI_BASE_URL");
        return Ok(());
    };
    run_cli(
        &base_url,
        &[
            "run",
            "--agent",
            "distri_runner",
            "--remote",
            "--task",
            "Create a Python script that builds a pandas DataFrame with this sales data: \
             Apple=150 units at $1.20, Banana=230 at $0.50, Cherry=89 at $2.50, \
             Dragonfruit=310 at $4.00, Elderberry=175 at $3.75. Calculate total revenue per \
             product (units * price), find the top product by revenue, and print a plain-text bar \
             chart of units sold using only dashes (no matplotlib). Return all results as text.",
        ],
    )?;
    Ok(())
}

/// Remote run with a simple shell command.
///
/// Verifies the container can execute basic bash commands.
#[test]
#[ignore]
fn remote_shell_command() -> Result<()> {
    let _lock = STREAM_LOCK.lock().unwrap();
    let Some(base_url) = base_url() else {
        eprintln!("Skipping; set DISTRI_BASE_URL");
        return Ok(());
    };
    let output = run_cli(
        &base_url,
        &[
            "run",
            "--agent",
            "distri_runner",
            "--remote",
            "--task",
            "Run `uname -a` and return the output.",
        ],
    )?;

    // The agent should return some output — not empty
    assert!(
        !output.trim().is_empty(),
        "remote shell command should produce output"
    );
    Ok(())
}

/// Remote run with a simple arithmetic task (no tools needed beyond `final`).
#[test]
#[ignore]
fn remote_simple_arithmetic() -> Result<()> {
    let _lock = STREAM_LOCK.lock().unwrap();
    let Some(base_url) = base_url() else {
        eprintln!("Skipping; set DISTRI_BASE_URL");
        return Ok(());
    };
    run_cli(
        &base_url,
        &[
            "run",
            "--agent",
            "distri_runner",
            "--remote",
            "--task",
            "What is 7 * 6? Return only the number.",
        ],
    )?;
    Ok(())
}
