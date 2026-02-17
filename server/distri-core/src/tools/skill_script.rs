use std::sync::Arc;

use distri_types::stores::SkillScriptRecord;
use distri_types::{Part, ToolCall, tool::ToolContext};

#[derive(Debug)]
pub struct SkillScriptTool {
    skill_name: String,
    script: SkillScriptRecord,
}

impl SkillScriptTool {
    pub fn new(skill_name: String, script: SkillScriptRecord) -> Self {
        Self { skill_name, script }
    }
}

#[async_trait::async_trait]
impl distri_types::Tool for SkillScriptTool {
    fn get_name(&self) -> String {
        format!(
            "skill_{}_{}",
            self.skill_name.replace(' ', "_").to_lowercase(),
            self.script.name.replace(' ', "_").to_lowercase()
        )
    }

    fn get_description(&self) -> String {
        self.script.description.clone().unwrap_or_else(|| {
            format!(
                "Execute {} script from skill {}",
                self.script.name, self.skill_name
            )
        })
    }

    fn get_parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Input data or arguments for the script"
                }
            }
        })
    }

    async fn execute(
        &self,
        _tool_call: ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        let result = format!(
            "## Script: {} ({})\n\n```{}\n{}\n```\n\nThis script is ready for execution.",
            self.script.name, self.script.language, self.script.language, self.script.code
        );
        Ok(vec![Part::Text(result)])
    }
}
