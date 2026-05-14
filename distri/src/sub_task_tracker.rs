//! Sub-task fence printer for the CLI.
//!
//! The CLI streams every event flat — root-task assistant text, sub-task
//! tool calls, sub-sub-task progress — into one column of output, which
//! is unreadable once a run dispatches more than one helper.
//!
//! Every `AgentEvent` carries `task_id` and `parent_task_id` (set by the
//! orchestrator on dispatch). This tracker walks that tree as events
//! arrive and emits fence headers / footers when execution crosses a
//! task boundary, so the user sees:
//!
//! ```text
//! ◆ assistant: dispatching a researcher…
//! ┌─ subtask · researcher ────────────────────
//! │ ⏺ search_web(query="…")
//! │ ⎿ 5 results
//! │ ◆ researcher: here is what I found…
//! └─ ✓ researcher (3.4s)
//! ◆ assistant: synthesising…
//! ```
//!
//! Output for the sub-task body itself stays exactly as the existing
//! `EventPrinter` writes it — the tracker only prints fence lines on
//! task transitions. That keeps the prototype additive: no rewrite of
//! the streaming text path.

use std::collections::HashMap;
use std::time::Instant;

use distri_types::{AgentEvent, AgentEventType};

use distri_formatter::colors::{COLOR_BRIGHT_CYAN, COLOR_GRAY, COLOR_RED, COLOR_RESET};

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct TaskNode {
    task_id: String,
    parent_task_id: Option<String>,
    agent_id: String,
    depth: usize,
    started_at: Option<Instant>,
    header_printed: bool,
    footer_printed: bool,
}

/// Tracks task lineage from event stream and prints fence headers/footers
/// when execution crosses sub-task boundaries.
#[derive(Default)]
pub struct SubTaskTracker {
    tasks: HashMap<String, TaskNode>,
    /// Last task whose event we processed. Used to detect transitions.
    last_task_id: Option<String>,
    /// Root task id (first task we see without a parent). Anything else
    /// is a sub-task whose entry should print a header.
    root_task_id: Option<String>,
}

#[allow(dead_code)]
impl SubTaskTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Walk the parent chain from `task_id` up to a known root to compute
    /// depth. Returns 0 if `task_id` is the root or unknown.
    fn depth_of(&self, task_id: &str) -> usize {
        let mut cur = task_id.to_string();
        let mut depth = 0;
        while let Some(node) = self.tasks.get(&cur) {
            match &node.parent_task_id {
                Some(p) => {
                    depth += 1;
                    cur = p.clone();
                }
                None => break,
            }
        }
        depth
    }

    fn indent(depth: usize) -> String {
        // One bar per nesting level, e.g. depth=2 → "│ │ "
        let mut s = String::new();
        for _ in 0..depth {
            s.push_str("│ ");
        }
        s
    }

    /// Called for every event before the underlying printer handles it.
    /// Returns the (depth, transition_hint) so the printer can prefix its
    /// lines with the right indent and the caller can decide what to do
    /// on the fence boundary.
    pub fn observe(&mut self, event: &AgentEvent) {
        let task_id = event.task_id.clone();
        if task_id.is_empty() {
            return;
        }

        // Insert / backfill the task node. Compute the depth up front
        // so the entry closure doesn't need a second borrow of `self`.
        let parent_task_id = event.parent_task_id.clone();
        let computed_depth = parent_task_id
            .as_ref()
            .and_then(|p| self.tasks.get(p).map(|n| n.depth + 1))
            .unwrap_or(0);
        let node = self.tasks.entry(task_id.clone()).or_insert(TaskNode {
            task_id: task_id.clone(),
            parent_task_id: parent_task_id.clone(),
            agent_id: event.agent_id.clone(),
            depth: computed_depth,
            started_at: None,
            header_printed: false,
            footer_printed: false,
        });
        // Backfill agent / parent if first event lacked them.
        if node.agent_id.is_empty() && !event.agent_id.is_empty() {
            node.agent_id = event.agent_id.clone();
        }
        if node.parent_task_id.is_none() && parent_task_id.is_some() {
            node.parent_task_id = parent_task_id.clone();
            node.depth = computed_depth;
        }
        if self.root_task_id.is_none() && parent_task_id.is_none() {
            self.root_task_id = Some(task_id.clone());
        }

        // Print fence transitions on task switches.
        let transitioned = self
            .last_task_id
            .as_ref()
            .map(|prev| prev != &task_id)
            .unwrap_or(true);
        if transitioned {
            self.print_header_if_needed(&task_id, event);
        }
        self.last_task_id = Some(task_id.clone());

        // Capture start time once we see the first event for this task.
        if let Some(node) = self.tasks.get_mut(&task_id) {
            if node.started_at.is_none() {
                node.started_at = Some(Instant::now());
            }
        }

        // Print fence footer on this task's RunFinished / RunError.
        match &event.event {
            AgentEventType::RunFinished { success, .. } => {
                self.print_footer(&task_id, *success);
            }
            AgentEventType::RunError { .. } => {
                self.print_footer(&task_id, false);
            }
            _ => {}
        }
    }

    fn print_header_if_needed(&mut self, task_id: &str, event: &AgentEvent) {
        let node = match self.tasks.get_mut(task_id) {
            Some(n) => n,
            None => return,
        };
        if node.header_printed || node.depth == 0 {
            return;
        }
        let indent = Self::indent(node.depth.saturating_sub(1));
        let agent = if node.agent_id.is_empty() {
            "subtask"
        } else {
            &node.agent_id
        };
        // Pull a reason hint from RunStarted metadata if present.
        let reason = match &event.event {
            AgentEventType::RunStarted {} => String::new(),
            _ => String::new(),
        };
        println!(
            "{}{}┌─ subtask · {}{}{}",
            indent, COLOR_BRIGHT_CYAN, agent, reason, COLOR_RESET
        );
        node.header_printed = true;
    }

    fn print_footer(&mut self, task_id: &str, success: bool) {
        let node = match self.tasks.get_mut(task_id) {
            Some(n) => n,
            None => return,
        };
        if node.footer_printed || node.depth == 0 || !node.header_printed {
            return;
        }
        let indent = Self::indent(node.depth.saturating_sub(1));
        let elapsed = node
            .started_at
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        let glyph = if success { "✓" } else { "✖" };
        let color = if success { COLOR_GRAY } else { COLOR_RED };
        let agent = if node.agent_id.is_empty() {
            "subtask"
        } else {
            &node.agent_id
        };
        println!(
            "{}{}└─ {} {} ({:.1}s){}",
            indent, color, glyph, agent, elapsed, COLOR_RESET
        );
        node.footer_printed = true;
    }

    /// Indent prefix the caller can prepend to lines printed for the
    /// most recently observed event. Returns `""` for root-task events
    /// so existing output is unchanged.
    pub fn current_indent(&self) -> String {
        let task_id = match &self.last_task_id {
            Some(id) => id,
            None => return String::new(),
        };
        let depth = self
            .tasks
            .get(task_id)
            .map(|n| n.depth)
            .unwrap_or(0);
        if depth == 0 {
            return String::new();
        }
        // One bar per ancestor level. `│ │ ⏺ tool(...)`
        Self::indent(depth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::AgentEvent;

    fn ev(task_id: &str, parent: Option<&str>, agent: &str, event: AgentEventType) -> AgentEvent {
        AgentEvent {
            timestamp: chrono::Utc::now(),
            thread_id: "t".into(),
            run_id: "r".into(),
            event,
            task_id: task_id.into(),
            parent_task_id: parent.map(|s| s.into()),
            agent_id: agent.into(),
            user_id: None,
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
        }
    }

    #[test]
    fn depth_increases_with_parent_chain() {
        let mut t = SubTaskTracker::new();
        t.observe(&ev("root", None, "main", AgentEventType::RunStarted {}));
        t.observe(&ev("a", Some("root"), "alpha", AgentEventType::RunStarted {}));
        t.observe(&ev("b", Some("a"), "beta", AgentEventType::RunStarted {}));
        assert_eq!(t.depth_of("root"), 0);
        assert_eq!(t.depth_of("a"), 1);
        assert_eq!(t.depth_of("b"), 2);
    }

    #[test]
    fn root_event_has_empty_indent() {
        let mut t = SubTaskTracker::new();
        t.observe(&ev("root", None, "main", AgentEventType::RunStarted {}));
        assert_eq!(t.current_indent(), "");
    }
}
