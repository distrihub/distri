//! `distri integrate`: help while you wire an agent into your app.
//!
//! `distri integrate watch <agent>` follows the agent's calls as your app makes
//! them (the dashboard's Live tab, `GET /agents/{name}/live`). `--json` prints
//! one event per line for scripts and agents; `--calls N` stops after N calls
//! finish; `--strict` exits non-zero when a watched call had an issue.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{bail, Result};
use clap::Subcommand;
use distri::Distri;
use serde_json::Value;

#[derive(Subcommand, Debug, Clone)]
pub(crate) enum IntegrateCommands {
    /// Follow an agent's calls as they happen
    Watch {
        /// Agent name
        agent: String,
        /// Print each event as one line of JSON
        #[clap(long)]
        json: bool,
        /// Past events to show first
        #[clap(long, default_value = "0")]
        replay: usize,
        /// Stop after this many calls finish (0 = keep watching)
        #[clap(long, default_value = "0")]
        calls: usize,
        /// Give up after this many seconds (0 = no limit)
        #[clap(long, default_value = "0")]
        timeout: u64,
        /// Exit 1 if a watched call had an issue: a tool not declared, not sent,
        /// or failing, or the run failed
        #[clap(long)]
        strict: bool,
    },
}

pub(crate) async fn handle(client: &Distri, command: IntegrateCommands) -> Result<()> {
    match command {
        IntegrateCommands::Watch {
            agent,
            json,
            replay,
            calls,
            timeout,
            strict,
        } => {
            let mut watch = Watch::new(json);
            let run = client.watch_agent_live(&agent, replay, |event| {
                watch.on_event(&event);
                calls == 0 || watch.finished < calls
            });
            if !json {
                eprintln!("Watching {agent}. Use your app; calls show up here.");
            }
            let result = if timeout > 0 {
                match tokio::time::timeout(Duration::from_secs(timeout), run).await {
                    Ok(r) => r,
                    Err(_) => bail!(
                        "timed out after {timeout}s ({} calls finished)",
                        watch.finished
                    ),
                }
            } else {
                run.await
            };
            result?;
            if strict && watch.issues > 0 {
                bail!("{} call(s) had issues", watch.issues);
            }
            Ok(())
        }
    }
}

/// Prints the feed and counts finished calls and calls with issues.
struct Watch {
    json: bool,
    finished: usize,
    issues: usize,
    /// Calls with an issue so far, by task id.
    flagged: HashMap<String, bool>,
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn list(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn short(v: &Value) -> String {
    let text = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if text.chars().count() > 160 {
        format!("{}…", text.chars().take(160).collect::<String>())
    } else {
        text
    }
}

fn clock(v: &Value) -> String {
    v.get("at")
        .and_then(Value::as_i64)
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}

impl Watch {
    fn new(json: bool) -> Self {
        Self {
            json,
            finished: 0,
            issues: 0,
            flagged: HashMap::new(),
        }
    }

    fn flag(&mut self, task: &str) {
        self.flagged.insert(task.to_string(), true);
    }

    fn on_event(&mut self, e: &Value) {
        let task = s(e, "task_id").to_string();
        match s(e, "type") {
            "call_started" => {
                let undeclared = list(e, "undeclared");
                let missing = list(e, "missing");
                if !undeclared.is_empty() || !missing.is_empty() {
                    self.flag(&task);
                }
                if !self.json {
                    let caller = e.get("caller").cloned().unwrap_or_default();
                    let origin = s(&caller, "origin");
                    println!(
                        "{}  call  {}{}  sent: {}",
                        clock(e),
                        s(&caller, "kind"),
                        if origin.is_empty() {
                            String::new()
                        } else {
                            format!(" · {origin}")
                        },
                        list(e, "tools_sent").join(", ")
                    );
                    if !undeclared.is_empty() {
                        println!(
                            "          ! not declared on the agent: {}",
                            undeclared.join(", ")
                        );
                    }
                    if !missing.is_empty() {
                        println!("          ! not sent by the client: {}", missing.join(", "));
                    }
                }
            }
            "tool_call" if !self.json => {
                println!(
                    "          → {} {}",
                    s(e, "name"),
                    short(e.get("input").unwrap_or(&Value::Null))
                );
            }
            "tool_result" => {
                let error = s(e, "error").to_string();
                if !error.is_empty() {
                    let parent = e
                        .get("parent_task_id")
                        .and_then(Value::as_str)
                        .unwrap_or(&task)
                        .to_string();
                    self.flag(&parent);
                }
                if !self.json {
                    let ms = e
                        .get("ms")
                        .and_then(Value::as_u64)
                        .map(|m| format!(" {m} ms"))
                        .unwrap_or_default();
                    if error.is_empty() {
                        println!(
                            "          ← {}{}  {}",
                            s(e, "name"),
                            ms,
                            short(e.get("output").unwrap_or(&Value::Null))
                        );
                    } else {
                        println!("          ✗ {}{}  {}", s(e, "name"), ms, error);
                    }
                }
            }
            "call_finished" => {
                let ok = e.get("ok").and_then(Value::as_bool).unwrap_or(false);
                if !ok {
                    self.flag(&task);
                }
                if self.flagged.get(&task).copied().unwrap_or(false) {
                    self.issues += 1;
                }
                self.finished += 1;
                if !self.json {
                    if ok {
                        println!("          ✓ {}", s(e, "answer"));
                    } else {
                        println!("          ✗ failed: {}", s(e, "error"));
                    }
                }
            }
            _ => {}
        }
        if self.json {
            println!("{e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_finished_calls_and_calls_with_issues() {
        let mut w = Watch::new(true);
        w.on_event(&json!({ "type": "call_started", "task_id": "a", "undeclared": ["track_order"], "missing": [] }));
        w.on_event(&json!({ "type": "call_finished", "task_id": "a", "ok": true }));
        w.on_event(
            &json!({ "type": "call_started", "task_id": "b", "undeclared": [], "missing": [] }),
        );
        w.on_event(&json!({ "type": "tool_result", "task_id": "b", "name": "x", "error": "boom" }));
        w.on_event(&json!({ "type": "call_finished", "task_id": "b", "ok": true }));
        w.on_event(
            &json!({ "type": "call_started", "task_id": "c", "undeclared": [], "missing": [] }),
        );
        w.on_event(&json!({ "type": "call_finished", "task_id": "c", "ok": true }));
        assert_eq!(w.finished, 3);
        assert_eq!(w.issues, 2, "a (undeclared tool) and b (failed tool)");
    }
}
