use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn smoke_base_url() -> Option<String> {
    env::var("DISTRI_SMOKE_BASE_URL").ok()
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn run_cli(base_url: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(env!("CARGO_BIN_EXE_distri"))
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

fn ensure_smoke_base_url() -> Result<Option<String>> {
    let base_url = smoke_base_url();
    if base_url.is_none() {
        eprintln!("Skipping smoke test; set DISTRI_SMOKE_BASE_URL to enable.");
    }
    Ok(base_url)
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
    let Some(base_url) = ensure_smoke_base_url()? else {
        return Ok(());
    };
    run_cli(&base_url, &["agents", "list"])
}

#[test]
fn smoke_agents_push_single() -> Result<()> {
    let Some(base_url) = ensure_smoke_base_url()? else {
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
    let Some(base_url) = ensure_smoke_base_url()? else {
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
    let Some(base_url) = ensure_smoke_base_url()? else {
        return Ok(());
    };
    run_cli(&base_url, &["tools", "list"])
}

#[test]
fn smoke_tools_invoke() -> Result<()> {
    let Some(base_url) = ensure_smoke_base_url()? else {
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
