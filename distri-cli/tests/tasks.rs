//! `distri agents card`, `distri run --json|--stream` and `distri tasks …`
//! against a running server (skipped unless DISTRI_BASE_URL is set).
//!
//!   DISTRI_BASE_URL          e.g. http://127.0.0.1:8081/v1 (an OSS `distri-server`)
//!   DISTRI_A2A_TEST_AGENT    agent to talk to (default: distri); its model must answer
//!   DISTRI_A2A_EXPECT_DURABLE=1  the server runs with DISTRI_DURABLE_RUNS=1
//!
//!   cargo test -p distri-cli --test tasks -- --test-threads=1
use anyhow::{Context, Result};
use serde_json::Value;
use std::process::Command;

const DURABLE_URI: &str = "https://distri.dev/a2a/extensions/durable-runs/v1";
const EVENT_STREAM_URI: &str = "https://distri.dev/a2a/extensions/event-stream/v1";

fn setup() -> Option<(String, String)> {
    let Ok(base) = std::env::var("DISTRI_BASE_URL") else {
        eprintln!("Skipping tasks CLI test; set DISTRI_BASE_URL to enable.");
        return None;
    };
    let agent = std::env::var("DISTRI_A2A_TEST_AGENT").unwrap_or_else(|_| "distri".into());
    Some((base, agent))
}

/// Run the CLI; (stdout, stderr).
fn distri_io(base: &str, args: &[&str]) -> Result<(String, String)> {
    let out = Command::new(env!("CARGO_BIN_EXE_distri"))
        .arg("--base-url")
        .arg(base)
        .args(args)
        .env("DISTRI_LOG", "warn")
        .output()
        .with_context(|| format!("running distri {args:?}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        anyhow::bail!(
            "distri {args:?} failed ({}):\n{stdout}\n{stderr}",
            out.status
        );
    }
    Ok((stdout, stderr))
}

fn distri(base: &str, args: &[&str]) -> Result<String> {
    distri_io(base, args).map(|(o, _)| o)
}

#[test]
fn agents_card_is_a2a_1_0_with_extensions() -> Result<()> {
    let Some((base, agent)) = setup() else {
        return Ok(());
    };
    let raw: Value = serde_json::from_str(&distri(&base, &["agents", "card", &agent, "--json"])?)?;
    assert_eq!(raw["supportedInterfaces"][0]["protocolVersion"], "1.0");
    let uris: Vec<&str> = raw["capabilities"]["extensions"]
        .as_array()
        .map(|e| e.iter().filter_map(|x| x["uri"].as_str()).collect())
        .unwrap_or_default();
    assert!(uris.contains(&EVENT_STREAM_URI), "{uris:?}");
    let summary = distri(&base, &["agents", "card", &agent])?;
    assert!(summary.contains("Capabilities:"), "{summary}");
    assert!(summary.contains("(A2A 1.0)"), "{summary}");
    if std::env::var("DISTRI_A2A_EXPECT_DURABLE").as_deref() == Ok("1") {
        assert!(uris.contains(&DURABLE_URI), "card: {raw}");
        assert!(summary.contains(DURABLE_URI), "{summary}");
    }
    Ok(())
}

#[test]
fn run_json_then_tasks_get_and_list_round_trip() -> Result<()> {
    let Some((base, agent)) = setup() else {
        return Ok(());
    };
    let context = uuid::Uuid::new_v4().to_string();
    let sent: Value = serde_json::from_str(&distri(
        &base,
        &[
            "run",
            "--agent",
            &agent,
            "--task",
            "hello",
            "--context-id",
            &context,
            "--json",
        ],
    )?)?;
    let task = &sent["task"];
    let task_id = task["id"].as_str().context("SendMessage returns a task")?;
    assert_eq!(task["contextId"], context.as_str());

    let got: Value = serde_json::from_str(&distri(
        &base,
        &["tasks", "get", task_id, "--agent", &agent, "--json"],
    )?)?;
    assert_eq!(got["id"], task_id);

    let listed: Value = serde_json::from_str(&distri(
        &base,
        &["tasks", "list", "--context", &context, "--json"],
    )?)?;
    let ids: Vec<&str> = listed["tasks"]
        .as_array()
        .context("ListTasks returns tasks")?
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert!(ids.contains(&task_id), "{ids:?}");

    let table = distri(&base, &["tasks", "list", "--context", &context])?;
    assert!(table.contains(task_id), "{table}");

    // Standard A2A stream: task first, terminal status, final task.
    let streamed = distri(
        &base,
        &[
            "run",
            "--agent",
            &agent,
            "--task",
            "hello again",
            "--context-id",
            &context,
            "--stream",
        ],
    )?;
    assert!(
        streamed.lines().next().unwrap_or("").starts_with("[task]"),
        "{streamed}"
    );
    assert!(streamed.contains("[status] completed"), "{streamed}");

    // event-stream activated: distri events, echoed on stderr.
    let (streamed, stderr) = distri_io(
        &base,
        &[
            "run",
            "--agent",
            &agent,
            "--task",
            "and again",
            "--context-id",
            &context,
            "--stream",
            "--extensions",
            "event-stream",
        ],
    )?;
    assert!(
        streamed.lines().any(|l| l.contains("run_started")),
        "{streamed}"
    );
    assert!(streamed.contains("(final)"), "{streamed}");
    assert!(stderr.contains(EVENT_STREAM_URI), "{stderr}");
    Ok(())
}
