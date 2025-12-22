use distri_types::{Action, AgentError, AgentPlan, PlanStep, ToolCall};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Unified agent response structure supporting CoT, ReAct, and hybrid patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct AgentResponse {
    /// Reasoning/thought - always present
    pub thought: Option<String>,
    /// Action to take - optional (None for pure CoT reasoning steps)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Parameters for the action - optional
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}

impl AgentResponse {
    pub fn new(
        thought: Option<String>,
        action: Option<String>,
        parameters: Option<serde_json::Value>,
    ) -> Self {
        Self {
            thought,
            action,
            parameters,
        }
    }

    pub fn from_xml(xml_str: &str) -> Result<Self, Box<dyn std::error::Error>> {
        // Extract XML from markdown code blocks if present
        let xml_content = Self::find_xml_block(xml_str).unwrap_or(xml_str);

        // Debug: Log the XML content being parsed
        tracing::debug!("Parsing XML content:\n{}", xml_content);

        let doc = roxmltree::Document::parse(xml_content).map_err(|e| {
            tracing::error!("XML parsing error: {} - Content:\n{}", e, xml_content);
            e
        })?;
        let root = doc.root_element();

        // Extract thought
        let thought = root
            .children()
            .find(|n| n.has_tag_name("thought"))
            .and_then(|n| n.text())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        // Extract action
        let action = root
            .children()
            .find(|n| n.has_tag_name("action"))
            .and_then(|n| n.text())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        // Extract parameters - expect perfect JSON
        let parameters = root
            .children()
            .find(|n| n.has_tag_name("parameters"))
            .and_then(|n| n.text())
            .and_then(|params_text| {
                let trimmed = params_text.trim();
                serde_json::from_str::<serde_json::Value>(trimmed).ok()
            });

        Ok(AgentResponse {
            thought,
            action,
            parameters,
        })
    }

    /// Extract XML content from markdown code blocks
    fn find_xml_block(text: &str) -> Option<&str> {
        // This regex matches a markdown code block with xml, e.g. ```xml ... ```
        let re = Regex::new(r"```xml\s*([\s\S]*?)\s*```").unwrap();
        re.captures(text)
            .and_then(|caps| caps.get(1).map(|m| m.as_str()))
    }
    /// Parse LLM response into plan steps based on strategy configuration
    pub async fn parse(response: &str) -> Result<AgentPlan, AgentError> {
        // Try to parse XML response for all modes (tools execution and mixed mode)
        if let Ok(agent_resp) = AgentResponse::from_xml(response.trim()) {
            let mut steps = Vec::new();

            // Add the action step if we have a valid action (execution cycle)
            if let Some(action) = agent_resp.action {
                if !action.is_empty() {
                    let input = match agent_resp.parameters {
                        Some(params) => params,
                        None => serde_json::json!({}),
                    };

                    steps.push(PlanStep {
                        id: uuid::Uuid::new_v4().to_string(),
                        action: Action::ToolCalls {
                            tool_calls: vec![ToolCall {
                                tool_call_id: uuid::Uuid::new_v4().to_string(),
                                tool_name: action,
                                input,
                            }],
                        },
                        thought: agent_resp.thought.clone(),
                    });
                }
            }

            return Ok(AgentPlan {
                steps,
                reasoning: None,
            });
        }

        // Fallback: create a simple thought step that contains the LLM response
        return Err(AgentError::Planning(format!(
            "Failed to parse response into plan steps: {}",
            response
        )));
    }
}
