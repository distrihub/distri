//! One way to ask the person anything: the [`Broker`].
//!
//! Every local tool that needs a person — an `approval_request`, a question
//! an agent asks, a script tool's first-use grant, a `Bash`/`Write`/`Edit`
//! permission — sends an [`Interaction`] and awaits a [`Reply`]. The TUI
//! answers through [`ChannelBroker`] (an inline prompt in its live region);
//! a plain terminal answers through [`TerminalBroker`] (a line on stdin);
//! with nobody to ask, [`NoBroker`] says [`Reply::Unavailable`].
//!
//! Nothing outside this module reads stdin to ask a question.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use distri_types::ToolCall;
use tokio::sync::{mpsc, oneshot};

/// What the person is asked.
#[derive(Debug, Clone, PartialEq)]
pub enum Interaction {
    /// An `approval_request` from the agent loop: may these calls run?
    Approve {
        calls: Vec<ToolCall>,
        reason: Option<String>,
    },
    /// An external call only a person can answer (e.g. `ask_human`).
    Answer {
        agent_id: String,
        call: ToolCall,
        /// The human-readable question, pulled from the call's input.
        question: String,
    },
    /// A script tool asks for sandbox access beyond its folder.
    Grant {
        tool: String,
        script: PathBuf,
        /// One line: `net: a, b; read: ./x`.
        request: String,
    },
    /// A local built-in (`Bash`, `Write`, `Edit`, …) or a script tool needs
    /// permission under the current mode and rules.
    Permit {
        tool: String,
        /// What the call does, for people: the command, the path.
        summary: String,
        /// The rule "don't ask again" would add, e.g. `Bash(cargo test:*)`.
        rule_hint: Option<String>,
    },
}

impl Interaction {
    /// The tool the interaction is about, for titles.
    pub fn tool_name(&self) -> &str {
        match self {
            Interaction::Approve { calls, .. } => calls
                .first()
                .map(|c| c.tool_name.as_str())
                .unwrap_or("approval_request"),
            Interaction::Answer { call, .. } => &call.tool_name,
            Interaction::Grant { tool, .. } | Interaction::Permit { tool, .. } => tool,
        }
    }
}

/// What the person said.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// Yes. `remember`: don't ask again (a grant is stored, a rule added).
    Allow { remember: bool },
    /// No, optionally with what to tell the agent instead.
    Deny { reason: Option<String> },
    /// The text answer to an [`Interaction::Answer`].
    Text(String),
    /// Nobody could be asked (no terminal, the TUI closed). Treat as a deny
    /// that is not remembered.
    Unavailable,
}

impl Reply {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Reply::Allow { .. })
    }
}

/// Asks the person.
#[async_trait]
pub trait Broker: Send + Sync {
    async fn ask(&self, interaction: Interaction) -> Reply;
}

pub type SharedBroker = Arc<dyn Broker>;

/// A question in flight to the TUI.
pub struct Ask {
    pub interaction: Interaction,
    pub reply: oneshot::Sender<Reply>,
}

impl std::fmt::Debug for Ask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ask({:?})", self.interaction)
    }
}

/// Sends each question to the TUI's event loop and awaits its answer.
#[derive(Clone)]
pub struct ChannelBroker {
    tx: mpsc::Sender<Ask>,
}

impl ChannelBroker {
    pub fn new(buffer: usize) -> (Self, mpsc::Receiver<Ask>) {
        let (tx, rx) = mpsc::channel(buffer);
        (Self { tx }, rx)
    }
}

#[async_trait]
impl Broker for ChannelBroker {
    async fn ask(&self, interaction: Interaction) -> Reply {
        let (reply, rx) = oneshot::channel();
        if self.tx.send(Ask { interaction, reply }).await.is_err() {
            return Reply::Unavailable;
        }
        rx.await.unwrap_or(Reply::Unavailable)
    }
}

/// Nobody to ask: every question is [`Reply::Unavailable`].
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBroker;

#[async_trait]
impl Broker for NoBroker {
    async fn ask(&self, _interaction: Interaction) -> Reply {
        Reply::Unavailable
    }
}

/// Asks on stderr and reads one line from stdin, for `distri run` in a
/// plain terminal. Without a terminal on stdin it is [`NoBroker`].
#[derive(Debug, Clone, Copy, Default)]
pub struct TerminalBroker;

#[async_trait]
impl Broker for TerminalBroker {
    async fn ask(&self, interaction: Interaction) -> Reply {
        if !std::io::stdin().is_terminal() {
            return Reply::Unavailable;
        }
        tokio::task::spawn_blocking(move || ask_on_terminal(&interaction))
            .await
            .unwrap_or(Reply::Unavailable)
    }
}

fn ask_on_terminal(interaction: &Interaction) -> Reply {
    let mut err = std::io::stderr();
    let prompt = match interaction {
        Interaction::Approve { calls, reason } => {
            let names: Vec<&str> = calls.iter().map(|c| c.tool_name.as_str()).collect();
            format!(
                "\nApprove {}{}? [y/N] ",
                names.join(", "),
                reason
                    .as_deref()
                    .map(|r| format!(" ({r})"))
                    .unwrap_or_default()
            )
        }
        Interaction::Answer { question, .. } => format!("\n{question}\n> "),
        Interaction::Grant {
            tool,
            script,
            request,
        } => format!(
            "\nScript tool '{tool}' ({}) asks for {request}.\nAllow? [y = once / a = always for this project / N] ",
            script.display()
        ),
        Interaction::Permit {
            tool,
            summary,
            rule_hint,
        } => format!(
            "\n{tool}: {summary}\nAllow? [y = once{} / N] ",
            rule_hint
                .as_deref()
                .map(|r| format!(" / a = always ({r})"))
                .unwrap_or_default()
        ),
    };
    let _ = write!(err, "{prompt}");
    let _ = err.flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return Reply::Unavailable;
    }
    let line = line.trim_end_matches(['\n', '\r']).to_string();
    match interaction {
        Interaction::Answer { .. } => Reply::Text(line),
        _ => match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => Reply::Allow { remember: false },
            "a" | "always" => Reply::Allow { remember: true },
            _ => Reply::Deny { reason: None },
        },
    }
}

/// The question an `Answer` interaction shows: the first of `question`,
/// `prompt`, `message` or `text` that is a string in the call's input, else
/// the input as pretty JSON.
pub fn question_of(call: &ToolCall) -> String {
    for key in ["question", "prompt", "message", "text"] {
        if let Some(s) = call.input.get(key).and_then(|v| v.as_str()) {
            return s.to_string();
        }
    }
    match &call.input {
        serde_json::Value::String(s) => s.clone(),
        v => serde_json::to_string_pretty(v).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(input: serde_json::Value) -> ToolCall {
        ToolCall {
            tool_call_id: "c1".into(),
            tool_name: "ask_human".into(),
            input,
        }
    }

    #[test]
    fn question_prefers_named_fields_then_json() {
        assert_eq!(
            question_of(&call(serde_json::json!({"question": "Ship Friday?"}))),
            "Ship Friday?"
        );
        assert_eq!(
            question_of(&call(serde_json::json!({"prompt": "Name?"}))),
            "Name?"
        );
        assert!(question_of(&call(serde_json::json!({"x": 1}))).contains("\"x\": 1"));
    }

    #[tokio::test]
    async fn channel_broker_round_trips_and_reports_a_closed_ui() {
        let (broker, mut rx) = ChannelBroker::new(4);
        let task = tokio::spawn({
            let broker = broker.clone();
            async move {
                broker
                    .ask(Interaction::Permit {
                        tool: "Bash".into(),
                        summary: "ls".into(),
                        rule_hint: None,
                    })
                    .await
            }
        });
        let ask = rx.recv().await.expect("asked");
        assert_eq!(ask.interaction.tool_name(), "Bash");
        ask.reply.send(Reply::Allow { remember: true }).unwrap();
        assert_eq!(task.await.unwrap(), Reply::Allow { remember: true });

        drop(rx);
        let reply = broker
            .ask(Interaction::Permit {
                tool: "Bash".into(),
                summary: "ls".into(),
                rule_hint: None,
            })
            .await;
        assert_eq!(reply, Reply::Unavailable);
    }
}
