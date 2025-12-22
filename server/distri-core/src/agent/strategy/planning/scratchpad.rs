use distri_types::{Action, Part};
use distri_types::{ScratchpadEntry, ScratchpadEntryType};

/// Format scratchpad entries with optional task_id filtering
/// This allows filtering entries by task_id to keep separate agent contexts
pub fn format_scratchpad_with_task_filter(
    entries: &[ScratchpadEntry],
    limit: Option<usize>,
    task_id_filter: Option<&str>,
) -> String {
    if entries.is_empty() {
        return String::new();
    }

    // Apply task_id filtering first if specified
    let filtered_entries: Vec<&ScratchpadEntry> = if let Some(filter_task_id) = task_id_filter {
        entries
            .iter()
            .filter(|entry| {
                match &entry.entry_type {
                    ScratchpadEntryType::Execution(exec_entry) => {
                        exec_entry.task_id == filter_task_id
                    }
                    // Include plan steps and tasks for all agents for context
                    ScratchpadEntryType::PlanStep(_) | ScratchpadEntryType::Task(_) => true,
                }
            })
            .collect()
    } else {
        entries.iter().collect()
    };

    // Apply limit if specified (keep most recent)
    let entries_to_use: Vec<&ScratchpadEntry> = if let Some(limit) = limit {
        filtered_entries
            .into_iter()
            .rev()
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    } else {
        filtered_entries
    };

    // Format scratchpad with proper ReAct structure
    let mut scratchpad = String::new();
    let mut current_task_id: Option<String> = None;

    for entry in entries_to_use.iter() {
        match &entry.entry_type {
            ScratchpadEntryType::Task(task) => {
                let task_text = task
                    .iter()
                    .filter_map(|part| match part {
                        Part::Text(text) => Some(text),
                        _ => None,
                    })
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");
                if !task_text.is_empty() {
                    scratchpad.push_str(&format!("Task: {}\n", task_text));
                }
            }
            ScratchpadEntryType::PlanStep(step) => {
                // Format plan step based on its kind - this gives us Thoughts and Actions
                if let Some(thought) = &step.thought {
                    scratchpad.push_str(&format!("Thought: {}\n", thought));
                }

                match &step.action {
                    Action::ToolCalls { tool_calls } => {
                        for tool_call in tool_calls {
                            scratchpad.push_str(&format!(
                                "Action: Call {} tool with input: {}\n",
                                tool_call.tool_name, tool_call.input
                            ));
                        }
                    }
                    Action::Code { code, .. } => {
                        scratchpad.push_str(&format!("Action: Execute code\n```\n{}\n```\n", code));
                    }
                }
            }
            ScratchpadEntryType::Execution(exec_entry) => {
                // Add task separator when task changes
                if current_task_id.as_ref() != Some(&exec_entry.task_id) {
                    if current_task_id.is_some() {
                        scratchpad.push_str("\n---\n\n");
                    }
                    current_task_id = Some(exec_entry.task_id.clone());
                }

                // Add execution result as observation
                scratchpad.push_str(&format!(
                    "Observation: {}\n",
                    exec_entry.execution_result.as_observation()
                ));
            }
        }
    }

    scratchpad
}
