use distri::Distri;
use distri_types::{ToolResponse, Part};
use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;

/// State file for active sidecar session.
/// Written by `distri sidecar start`, read by `distri sidecar send`.
fn state_file_path() -> PathBuf {
    dirs::home_dir()
        .expect("cannot determine home directory")
        .join(".distri")
        .join("sidecar-state.json")
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
struct SidecarState {
    thread_id: String,
    task_id: String,
    agent_name: String,
    base_url: String,
}

fn read_state() -> Option<SidecarState> {
    let path = state_file_path();
    let contents = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&contents).ok()
}

fn write_state(state: &SidecarState) {
    let path = state_file_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, serde_json::to_string_pretty(state).unwrap());
}

/// Run `distri sidecar start --task <message> [--agent-name claude]`
///
/// Creates a sidecar session on distri-cloud (thread + task), then writes
/// session info to ~/.distri/sidecar-state.json so `distri sidecar send`
/// can send events to it.
pub async fn run_start(task: &str, agent_name: Option<&str>) {
    let agent_name = agent_name
        .map(|s| s.to_string())
        .or_else(|| std::env::var("DISTRI_AGENT_NAME").ok())
        .unwrap_or_else(|| "claude".to_string());

    let mut config = crate::credentials::load_config_with_profile();
    if let Ok(base_url) = std::env::var("DISTRI_BASE_URL") {
        config.base_url = base_url.trim_end_matches('/').to_string();
    }

    let client = Distri::from_config(config);

    // POST /v1/sidecar/session
    let url = format!(
        "{}/sidecar/session",
        client.config().base_url.trim_end_matches('/')
    );

    let body = serde_json::json!({
        "agent_name": agent_name,
        "title": task.to_string(),
    });

    let api_key = client.config().api_key.clone().unwrap_or_default();

    let resp = match reqwest::Client::new()
        .post(&url)
        .header("Content-Type", "application/json")
        .bearer_auth(&api_key)
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("sidecar: failed to create session: {e}");
            std::process::exit(1);
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        eprintln!("sidecar: session creation failed ({status}): {body}");
        std::process::exit(1);
    }

    let session: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("sidecar: failed to parse session response: {e}");
            std::process::exit(1);
        }
    };

    let thread_id = session["thread_id"].as_str().unwrap_or_default().to_string();
    let task_id = session["task_id"].as_str().unwrap_or_default().to_string();

    let state = SidecarState {
        thread_id,
        task_id,
        agent_name: agent_name.clone(),
        base_url: client.config().base_url.clone(),
    };

    write_state(&state);

    println!("Sidecar session started.");
    println!("  Thread: {}", state.thread_id);
    println!("  Task:   {}", state.task_id);
    println!("  State:  {}", state_file_path().display());
    println!();
    println!("Now run Claude Code with hooks configured. Events will be");
    println!("recorded to this session automatically.");
    println!();
    println!("Press Ctrl+C when done.");
    println!();

    // Keep running until Ctrl+C — user can run Claude in another terminal
    tokio::signal::ctrl_c().await.ok();
    println!("Sidecar session ended.");
}

/// Run `distri sidecar send --event <type> [--agent-name claude]`
///
/// Reads Claude Code hook JSON from stdin, translates to AgentEvent format,
/// and sends it to the distri-cloud sidecar event endpoint.
/// Always exits 0 (best-effort, never blocks Claude Code).
pub async fn run_send(
    event_type: &str,
    agent_name: Option<&str>,
) {
    let state = read_state();

    let agent_name = agent_name
        .map(|s| s.to_string())
        .or_else(|| state.as_ref().map(|s| s.agent_name.clone()))
        .or_else(|| std::env::var("DISTRI_AGENT_NAME").ok())
        .unwrap_or_else(|| "claude".to_string());

    let base_url = state
        .as_ref()
        .map(|s| s.base_url.clone())
        .or_else(|| std::env::var("DISTRI_BASE_URL").ok())
        .unwrap_or_default();

    // Read hook JSON from stdin (Claude Code passes it on stdin)
    let mut stdin_payload = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut stdin_payload) {
        eprintln!("sidecar: failed to read stdin: {e}");
        return;
    }

    let hook_payload: Value = match serde_json::from_str(&stdin_payload) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("sidecar: failed to parse stdin JSON: {e}");
            return;
        }
    };

    // If we have a state file, use the sidecar API (dumb mode)
    // Otherwise fall back to complete-tool (A2A smart mode)
    if let Some(ref state) = state {
        send_sidecar_event(&base_url, &state, event_type, &hook_payload).await;
    } else {
        eprintln!("sidecar: no active session found (run `distri sidecar start` first)");
    }
}

async fn send_sidecar_event(
    base_url: &str,
    state: &SidecarState,
    event_type: &str,
    hook_payload: &Value,
) {
    let mut config = crate::credentials::load_config_with_profile();
    config.base_url = base_url.trim_end_matches('/').to_string();

    let url = format!("{}/sidecar/event", config.base_url);

    let body = serde_json::json!({
        "thread_id": state.thread_id,
        "task_id": state.task_id,
        "event_type": event_type,
        "hook_payload": hook_payload,
    });

    let client = reqwest::Client::new();
    let _ = client
        .post(&url)
        .header("Content-Type", "application/json")
        .bearer_auth(config.api_key.unwrap_or_default())
        .json(&body)
        .send()
        .await;
}
