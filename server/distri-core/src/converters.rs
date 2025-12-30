use distri_types::{AgentPlan, TaskArtifactMetadata, TaskArtifactMetadataType, ToolCall, ToolResponse};

impl From<&ToolCall> for TaskArtifactMetadata {
    fn from(tool_call: &ToolCall) -> Self {
        Self::tool_call(tool_call)
    }
}

impl TaskArtifactMetadata {
    pub fn tool_call(tool_call: &ToolCall) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().timestamp_millis(),
            updated_at: chrono::Utc::now().timestamp_millis(),
            artifact: TaskArtifactMetadataType::ToolCall(tool_call.clone()),
        }
    }

    pub fn tool_result(tool_result: &ToolResponse) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().timestamp_millis(),
            updated_at: chrono::Utc::now().timestamp_millis(),
            artifact: TaskArtifactMetadataType::ToolResult(tool_result.clone()),
        }
    }

    pub fn plan(plan: &AgentPlan) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().timestamp_millis(),
            updated_at: chrono::Utc::now().timestamp_millis(),
            artifact: TaskArtifactMetadataType::Plan(plan.clone()),
        }
    }
}
