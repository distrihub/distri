//! `distri a2a …` against a running server (skipped unless DISTRI_BASE_URL is set).
//!
//!   DISTRI_BASE_URL          e.g. http://127.0.0.1:8081/v1 (an OSS `distri-server`)
//!   DISTRI_A2A_TEST_AGENT    agent to talk to (default: distri); its model must answer
//!   DISTRI_A2A_EXPECT_DURABLE=1  the server runs with DISTRI_DURABLE_RUNS=1
//!
//!   cargo test -p distri-cli --test a2a -- --test-threads=1
use anyhow::{Context, Result};
use serde_json::Value;
use std::process::Command;

const DURABLE_URI: &str = "https://distri.dev/a2a/extensions/durable-runs/v1";

fn setup() -> Option<(String, String)> {
    let Ok(base) = std::env::var("DISTRI_BASE_URL") else {
        eprintln!("Skipping a2a CLI test; set DISTRI_BASE_URL to enable.");
        return None;
    };
    let agent = std::env::var("DISTRI_A2A_TEST_AGENT").unwrap_or_else(|_| "distri".into());
    Some((base, agent))
}

fn distri(base: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(env!("CARGO_BIN_EXE_distri"))
        .arg("--base-url")
        .arg(base)
        .args(args)
        .env("DISTRI_LOG", "warn")
        .output()
        .with_context(|| format!("running distri {args:?}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        anyhow::bail!(
            "distri {args:?} failed ({}):\n{stdout}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(stdout)
}

#[test]
fn card_get_shows_the_durable_runs_extension() -> Result<()> {
    let Some((base, agent)) = setup() else {
        return Ok(());
    };
    let raw: Value = serde_json::from_str(&distri(
        &base,
        &["a2a", "card", "get", "--agent", &agent, "--json"],
    )?)?;
    assert!(raw.get("durable_runs").is_none(), "no custom card field");
    let has_ext = raw["capabilities"]["extensions"]
        .as_array()
        .map(|e| e.iter().any(|x| x["uri"] == DURABLE_URI))
        .unwrap_or(false);
    let summary = distri(&base, &["a2a", "card", "get", "--agent", &agent])?;
    assert!(summary.contains("Capabilities:"), "{summary}");
    if std::env::var("DISTRI_A2A_EXPECT_DURABLE").as_deref() == Ok("1") {
        assert!(has_ext, "card: {raw}");
        assert!(summary.contains(DURABLE_URI), "{summary}");
    }
    Ok(())
}

#[test]
fn send_then_task_get_and_list_round_trip() -> Result<()> {
    let Some((base, agent)) = setup() else {
        return Ok(());
    };
    let context = uuid::Uuid::new_v4().to_string();
    let sent: Value = serde_json::from_str(&distri(
        &base,
        &[
            "a2a",
            "send",
            "hello",
            "--agent",
            &agent,
            "--context",
            &context,
            "--json",
        ],
    )?)?;
    let task_id = sent["id"].as_str().context("message/send returns a task")?;
    assert_eq!(sent["contextId"], context.as_str());

    let got: Value = serde_json::from_str(&distri(
        &base,
        &["a2a", "task", "get", task_id, "--agent", &agent, "--json"],
    )?)?;
    assert_eq!(got["id"], task_id);

    let listed: Value = serde_json::from_str(&distri(
        &base,
        &["a2a", "task", "list", "--context", &context, "--json"],
    )?)?;
    let ids: Vec<&str> = listed
        .as_array()
        .context("task list is an array")?
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert!(ids.contains(&task_id), "{ids:?}");

    let table = distri(&base, &["a2a", "task", "list", "--context", &context])?;
    assert!(table.contains(task_id), "{table}");

    // Streaming prints one line per event and ends with the final status.
    let streamed = distri(
        &base,
        &[
            "a2a",
            "send",
            "hello again",
            "--agent",
            &agent,
            "--context",
            &context,
            "--stream",
        ],
    )?;
    assert!(
        streamed.lines().any(|l| l.contains("run_started")),
        "{streamed}"
    );
    assert!(streamed.contains("(final)"), "{streamed}");
    Ok(())
}
