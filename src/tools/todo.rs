use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::json;

use super::ToolError;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

#[derive(Debug, Deserialize)]
pub struct TodoWriteArgs {
    pub todos: Vec<TodoItem>,
}

#[derive(Debug, Clone, Default)]
pub struct TodoWriteTool {
    todos: std::sync::Arc<std::sync::Mutex<Vec<TodoItem>>>,
}

impl TodoWriteTool {
    /// Shared-state constructor: the agent owns the `Arc` and hands a
    /// clone to the tool so `call` mutates agent-visible state.
    pub fn with_shared(todos: std::sync::Arc<std::sync::Mutex<Vec<TodoItem>>>) -> Self {
        Self { todos }
    }
}

impl Tool for TodoWriteTool {
    const NAME: &'static str = "todowrite";
    type Args = TodoWriteArgs;
    type Output = String;
    type Error = ToolError;

    fn description(&self) -> String {
        "Owns the session task list. Pass the full list each call; each item has content + status (pending/in_progress/completed).".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "Full task list (replaces previous)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": { "type": "string" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                        },
                        "required": ["content", "status"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    async fn call(&self, _ctx: &mut ToolContext, args: Self::Args) -> Result<String, ToolError> {
        let mut guard = self
            .todos
            .lock()
            .map_err(|e| ToolError(format!("todowrite: list busy: {e}")))?;
        *guard = args.todos;
        let lines: Vec<String> = guard
            .iter()
            .map(|t| {
                let mark = match t.status {
                    TodoStatus::Pending => "○",
                    TodoStatus::InProgress => "◌",
                    TodoStatus::Completed => "●",
                };
                format!("{mark} {}", t.content)
            })
            .collect();
        match lines.is_empty() {
            true => Ok("todo list cleared".to_string()),
            false => Ok(lines.join("\n")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn todowrite_round_trip_persists_content_and_status() {
        let tool = TodoWriteTool::default();
        let mut ctx = ToolContext::new();
        let args: TodoWriteArgs = serde_json::from_value(serde_json::json!({
            "todos": [
                {"content": "write failing test", "status": "completed"},
                {"content": "make it pass", "status": "in_progress"},
                {"content": "ship it", "status": "pending"}
            ]
        }))
        .unwrap();
        let out = tool.call(&mut ctx, args).await.unwrap();
        assert!(out.contains("write failing test"), "got: {out}");
        assert!(out.contains("make it pass"), "got: {out}");
        assert!(out.contains("ship it"), "got: {out}");
    }

    #[tokio::test]
    async fn todowrite_ignores_unknown_priority_field() {
        // Claude-authored prompts carry `priority`; v1 accepts + ignores it.
        let tool = TodoWriteTool::default();
        let mut ctx = ToolContext::new();
        let args: TodoWriteArgs = serde_json::from_value(serde_json::json!({
            "todos": [
                {"content": "a", "status": "pending", "priority": "high"},
                {"content": "b", "status": "completed", "priority": "low"}
            ]
        }))
        .unwrap();
        let out = tool.call(&mut ctx, args).await.unwrap();
        assert!(out.contains("a"), "got: {out}");
        assert!(out.contains("b"), "got: {out}");
    }

    #[test]
    fn todowrite_rejects_bad_status() {
        let bad: Result<TodoWriteArgs, _> = serde_json::from_value(serde_json::json!({
            "todos": [{"content": "a", "status": "doing"}]
        }));
        assert!(bad.is_err(), "unknown status must fail to parse");
    }
}
