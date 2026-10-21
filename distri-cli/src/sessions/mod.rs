//! `distri sessions` — every run you have going, as a tree, and a way to
//! jump into one. Attach is A2A `tasks/resubscribe` (history, then live),
//! rendered by the same `EventPrinter` as `distri run`. Esc detaches and the
//! run keeps going. Also opened from `distri chat` with `/runs`.
//!
//! `state.rs` is the state machine (tested without a terminal), `ui.rs`
//! draws it, and this file is the loop that feeds it keys, list polls and
//! stream items and carries out the effects it returns.

mod state;
mod transcript;
mod ui;

use std::collections::HashMap;
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use distri::{AgentStreamClient, BuildHttpClient, Distri, DistriConfig, ExternalToolRegistry};
use distri_types::{RunListQuery, RunSummary};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub use state::Scope;
use state::{Effect, Input, Key, SessionsState};

/// How often the run list is refreshed.
const POLL: Duration = Duration::from_secs(2);

/// Open the switcher. `registry` answers external tool calls on attached
/// runs (the same registry `distri run` / `distri chat` use). Returns when
/// the person quits; the runs keep going.
pub async fn run_sessions(
    config: &DistriConfig,
    registry: ExternalToolRegistry,
    scope: Scope,
) -> Result<()> {
    let client = Distri::from_config(config.clone());
    if !std::io::stdout().is_terminal() {
        return print_plain(&client, &scope).await;
    }

    // Resubscribed streams stay open as long as the run lives.
    let http = config.clone().with_timeout(24 * 3600).build_http_client()?;
    let stream_client = Arc::new(
        AgentStreamClient::from_config(config.clone())
            .with_http_client(http)
            .with_tool_registry(registry),
    );
    let can_run = {
        let sc = stream_client.clone();
        Arc::new(move |tool: &str| sc.can_run_tool(tool)) as Arc<dyn Fn(&str) -> bool + Send + Sync>
    };
    let mut state = SessionsState::new(scope, can_run);

    let (tx, mut rx) = mpsc::channel::<Input>(256);
    let stop = Arc::new(AtomicBool::new(false));
    let keys = spawn_key_reader(tx.clone(), stop.clone());
    let ticker = {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(POLL);
            every.tick().await;
            loop {
                every.tick().await;
                if tx.send(Input::Tick).await.is_err() {
                    break;
                }
            }
        })
    };

    let mut terminal = ratatui::init();
    let mut streams: HashMap<String, JoinHandle<()>> = HashMap::new();
    let mut pending = state.start();
    let result: Result<()> = async {
        loop {
            for effect in pending.drain(..) {
                if matches!(effect, Effect::Quit) {
                    return Ok(());
                }
                run_effect(effect, &client, &stream_client, &tx, &mut streams);
            }
            terminal.draw(|f| ui::draw(f, &state))?;
            let Some(input) = rx.recv().await else {
                return Ok(());
            };
            pending = state.handle(input).await;
        }
    }
    .await;

    stop.store(true, Ordering::Relaxed);
    ticker.abort();
    for (_, h) in streams.drain() {
        h.abort();
    }
    ratatui::restore();
    // The key reader polls every 100ms; wait for it so it can't swallow the
    // next keystroke meant for the chat prompt.
    let _ = keys.await;
    result
}

fn run_effect(
    effect: Effect,
    client: &Distri,
    stream_client: &Arc<AgentStreamClient>,
    tx: &mpsc::Sender<Input>,
    streams: &mut HashMap<String, JoinHandle<()>>,
) {
    let tx = tx.clone();
    match effect {
        Effect::LoadRuns(query) => {
            let client = client.clone();
            tokio::spawn(async move {
                let input = match client.list_runs(&query).await {
                    Ok(resp) => Input::RunsLoaded(resp),
                    Err(e) => Input::LoadFailed(e.to_string()),
                };
                let _ = tx.send(input).await;
            });
        }
        Effect::Attach { agent_id, task_id } => {
            let sc = stream_client.clone();
            let id = task_id.clone();
            let handle = tokio::spawn(async move {
                let items = tx.clone();
                let stream_id = id.clone();
                let res = sc
                    .resubscribe_task(&agent_id, &id, move |item| {
                        let items = items.clone();
                        let task_id = stream_id.clone();
                        async move {
                            let _ = items
                                .send(Input::Stream {
                                    task_id,
                                    item: Box::new(item),
                                })
                                .await;
                        }
                    })
                    .await;
                let _ = tx
                    .send(Input::StreamEnded {
                        task_id: id,
                        error: res.err().map(|e| e.to_string()),
                    })
                    .await;
            });
            if let Some(old) = streams.insert(task_id, handle) {
                old.abort();
            }
        }
        Effect::Detach { task_id } => {
            if let Some(h) = streams.remove(&task_id) {
                h.abort();
            }
        }
        Effect::RunTool {
            agent_id,
            event,
            call,
        } => {
            let sc = stream_client.clone();
            tokio::spawn(async move {
                let res = sc.answer_with_registry(&agent_id, &event, &call).await;
                let _ = tx
                    .send(Input::AnswerDone {
                        tool_call_id: call.tool_call_id,
                        error: res.err().map(|e| e.to_string()),
                    })
                    .await;
            });
        }
        Effect::SendText {
            agent_id,
            call,
            text,
        } => {
            let sc = stream_client.clone();
            tokio::spawn(async move {
                let res = sc.answer_with_text(&agent_id, &call, &text).await;
                let _ = tx
                    .send(Input::AnswerDone {
                        tool_call_id: call.tool_call_id,
                        error: res.err().map(|e| e.to_string()),
                    })
                    .await;
            });
        }
        Effect::Cancel { agent_id, task_id } => {
            let sc = stream_client.clone();
            tokio::spawn(async move {
                let res = sc.cancel_task(&agent_id, &task_id).await;
                let _ = tx
                    .send(Input::CancelDone {
                        task_id,
                        error: res.err().map(|e| e.to_string()),
                    })
                    .await;
            });
        }
        Effect::Quit => {}
    }
}

/// Terminal keys → [`Input::Key`], on a blocking thread, until `stop`.
fn spawn_key_reader(tx: mpsc::Sender<Input>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        while !stop.load(Ordering::Relaxed) {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(_) => break,
            }
            let Ok(Event::Key(k)) = event::read() else {
                continue;
            };
            if k.kind != KeyEventKind::Press {
                continue;
            }
            let key = match k.code {
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Key::CtrlC,
                KeyCode::Char(c) => Key::Char(c),
                KeyCode::Up => Key::Up,
                KeyCode::Down => Key::Down,
                KeyCode::PageUp => Key::PageUp,
                KeyCode::PageDown => Key::PageDown,
                KeyCode::Enter => Key::Enter,
                KeyCode::Esc => Key::Esc,
                KeyCode::Backspace => Key::Backspace,
                _ => continue,
            };
            if tx.blocking_send(Input::Key(key)).is_err() {
                break;
            }
        }
    })
}

/// No terminal (piped, CI): print the tree once.
async fn print_plain(client: &Distri, scope: &Scope) -> Result<()> {
    let query = RunListQuery {
        thread_id: match scope {
            Scope::Thread(t) => Some(t.clone()),
            Scope::All => None,
        },
        limit: Some(50),
        ..Default::default()
    };
    let resp = client.list_runs(&query).await?;
    if resp.runs.is_empty() {
        println!("No runs.");
    }
    for line in plain_lines(&resp.runs) {
        println!("{line}");
    }
    if resp.has_more {
        println!("… more runs not shown");
    }
    Ok(())
}

fn plain_lines(runs: &[RunSummary]) -> Vec<String> {
    runs.iter()
        .flat_map(|r| r.flatten())
        .map(|(depth, n)| {
            let (label, _) = ui::badge(n.state);
            format!(
                "{}{:<12} {:<24} {:>4}  {}  {}",
                "  ".repeat(depth),
                label,
                n.agent_id.as_deref().unwrap_or("?"),
                ui::age(n.latest_activity()),
                n.task_id,
                n.last_event.as_deref().unwrap_or("")
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::runs::fixture;

    #[test]
    fn plain_output_lists_the_tree() {
        let lines = plain_lines(&fixture::multi_run().runs);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("✓ done"));
        assert!(lines[1].starts_with("  ● needs you"));
        assert!(lines[1].contains(fixture::ASKER_AGENT));
        assert!(lines[2].contains(fixture::WORKER_AGENT));
    }
}
