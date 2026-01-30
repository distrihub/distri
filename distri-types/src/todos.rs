use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    #[serde(alias = "pending")]
    Open,
    #[serde(alias = "in_progress")]
    InProgress,
    #[serde(alias = "completed")]
    Done,
}

impl Default for TodoStatus {
    fn default() -> Self {
        TodoStatus::Open
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub notes: Option<String>,
    pub status: TodoStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TodoItem {
    pub fn new(title: String, notes: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            title,
            notes,
            status: TodoStatus::Open,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TodoList {
    pub items: Vec<TodoItem>,
}

impl TodoList {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn add(&mut self, title: String, notes: Option<String>) -> &TodoItem {
        let item = TodoItem::new(title, notes);
        self.items.push(item);
        self.items.last().unwrap()
    }

    pub fn update(
        &mut self,
        id: &str,
        title: Option<String>,
        notes: Option<String>,
        status: Option<TodoStatus>,
    ) -> Option<&TodoItem> {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            if let Some(t) = title {
                item.title = t;
            }
            if let Some(n) = notes {
                item.notes = Some(n);
            }
            if let Some(s) = status {
                item.status = s;
            }
            item.updated_at = Utc::now();
            return Some(item);
        }
        None
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let len_before = self.items.len();
        self.items.retain(|i| i.id != id);
        len_before != self.items.len()
    }

    /// Format todos for CLI / prompt display
    pub fn format_display(&self) -> String {
        if self.items.is_empty() {
            return "□ No todos".to_string();
        }

        self.items
            .iter()
            .map(|item| {
                let icon = match item.status {
                    TodoStatus::Done => "■",
                    TodoStatus::InProgress => "◐",
                    TodoStatus::Open => "□",
                };

                let mut line = format!("{} {}", icon, item.title.trim());
                if let Some(notes) = &item.notes {
                    let trimmed = notes.trim();
                    if !trimmed.is_empty() {
                        line.push_str(&format!(" ({})", trimmed));
                    }
                }

                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Get items by status
    pub fn get_by_status(&self, status: TodoStatus) -> Vec<&TodoItem> {
        self.items
            .iter()
            .filter(|item| item.status == status)
            .collect()
    }

    /// Check if there are any in-progress items
    pub fn has_in_progress(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.status == TodoStatus::InProgress)
    }

    /// Check if all items are completed
    pub fn is_all_completed(&self) -> bool {
        !self.items.is_empty()
            && self
                .items
                .iter()
                .all(|item| item.status == TodoStatus::Done)
    }

    /// Bulk replace todos with new list (similar to deepagents write_todos)
    pub fn write_todos(&mut self, simple_todos: Vec<SimpleTodo>) {
        self.items.clear();
        for simple_todo in simple_todos {
            let mut item = TodoItem::new(simple_todo.content, None);
            item.status = simple_todo.status;
            item.updated_at = chrono::Utc::now();
            self.items.push(item);
        }
    }
}

/// Thread-safe wrapper around TodoList for use in ExecutorContext
pub type SharedTodoList = Arc<tokio::sync::RwLock<TodoList>>;

/// Simple todo structure for bulk operations (similar to deepagents)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SimpleTodo {
    pub content: String,
    #[serde(default)]
    pub status: TodoStatus,
}

/// Tool parameters for todos operations - uses Serde for proper deserialization
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct TodoParams {
    pub todos: Option<Vec<SimpleTodo>>, // For bulk write_todos operations
}

/// Actions that can be performed on todos
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TodoAction {
    WriteTodos, // Bulk write/replace entire todo list (primary operation)
}

/// Response structure for todo operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoResponse {
    pub status: String,
    pub todos: TodoList,
    pub formatted: String,
}

impl TodoResponse {
    pub fn success(todos: TodoList) -> Self {
        Self {
            status: "ok".to_string(),
            formatted: todos.format_display(),
            todos,
        }
    }
}
