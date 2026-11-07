//! Events local tools emit while a call runs (progress, log lines), keyed by
//! the tool call. The server never sees them; the TUI shows them inside the
//! call's cell. A script tool's `ctx.progress` / `ctx.log` and its plain
//! stderr arrive here.

use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Log,
    Warn,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LocalToolEventKind {
    /// `fraction` in 0..=1 when known.
    Progress {
        fraction: Option<f64>,
        message: Option<String>,
    },
    Log {
        level: LogLevel,
        line: String,
    },
    /// The handler finished: what goes back to the agent, or the error it
    /// will read.
    Finished(Result<distri_types::ToolResponse, String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalToolEvent {
    pub tool_call_id: String,
    pub tool_name: String,
    pub kind: LocalToolEventKind,
}

/// Fan-out of [`LocalToolEvent`]s. Cheap to clone; sending with no
/// subscriber is a no-op.
#[derive(Debug, Clone)]
pub struct LocalEvents {
    tx: broadcast::Sender<LocalToolEvent>,
}

impl Default for LocalEvents {
    fn default() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { tx }
    }
}

impl LocalEvents {
    pub fn emit(&self, event: LocalToolEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LocalToolEvent> {
        self.tx.subscribe()
    }
}
