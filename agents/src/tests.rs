use std::sync::Arc;

use async_openai::types::{ChatCompletionRequestMessage, ChatCompletionRequestUserMessage, Role};
use serde_json::json;

use crate::{
    executor::AgentExecutor,
    init_logging,
    session::SessionStore,
    types::{
        AgentDefinition, AuthType, ModelSettings, Session, ToolDefinition, TransportType,
        UserMessage,
    },
};

static SYSTEM_PROMPT: &str = r#"You are a helpful AI assistant that can access Twitter and summarize information.
When asked about tweets, you will:
1. Get the timeline using the Twitter tool
2. Format the tweets in a clean markdown format
3. Add brief summaries and insights
4. Group similar tweets together by theme
5. Highlight particularly interesting or important tweets

Keep your summaries concise but informative. Use markdown formatting to make the output readable."#;

struct StaticSessionStore {
    session_key: String,
}

#[async_trait::async_trait]
impl SessionStore for StaticSessionStore {
    async fn save_session(&self, _tool_name: &str, _session: Session) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_session(&self, _tool_name: &str) -> anyhow::Result<Option<Session>> {
        Ok(Some(Session {
            token: self.session_key.clone(),
            expiry: None,
        }))
    }

    async fn delete_session(&self, _tool_name: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn test_twitter_summary() {
    init_logging("debug");

    // Create agent definition with Twitter tool
    let agent_def = AgentDefinition {
        name: "Twitter Agent".to_string(),
        description: "Agent that can access Twitter".to_string(),
        system_prompt: Some(SYSTEM_PROMPT.to_string()),
        model_settings: ModelSettings::default(),
        tools: vec![ToolDefinition {
            tool: mcp_sdk::types::Tool {
                name: "get_timeline".to_string(),
                description: Some("Get user's home timeline".to_string()),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "session_string": {"type": "string"},
                        "count": {"type": "integer", "default": 5}
                    },
                    "required": ["session_string"]
                }),
            },
            auth_type: AuthType::None,
            auth_session_key: Some("session_string".to_string()),
            mcp_transport: TransportType::Channel,
        }],
    };

    let session_key  = "guest_id_ads=v1%3A173719111551730639; kdt=T60Y7Gq1uM5JVTHqobkNQuxyAU2BOKlO8b3Gjzew; att=1-hS68dcUEf9FBYnfPFJKyG8UD1EWI0lHjsAYkU3xp; auth_token=c9d46f0b963dcaf2a2477e5b762c1abdcddabd95; personalization_id=v1_aUq4PsJLBR1VW/Rvsyi4ig==; guest_id_marketing=v1%3A173719111551730639; guest_id=v1%3A173719111551730639; twid=u=1497801936669913089; ct0=08ca694202f67ea16ac905516c64bf91838c6fe9e3f5680e66f1eac6c9d99f81aea56b1bd77964325d63a97dd86bce122b47d779d36221de420ea869fdd5f50fc5b33105373be8e45b695f991e01b3bb".to_string();
    // Create executor with static session store
    let session_store = Some(Arc::new(
        Box::new(StaticSessionStore { session_key }) as Box<dyn SessionStore>
    ));
    let executor = AgentExecutor::new(agent_def, session_store);

    let messages = vec![UserMessage {
        message: "Get my latest tweets and summarize them".to_string(),
        name: None,
    }];

    // Execute and print response
    let response = executor.execute(messages).await.unwrap();
    println!("Response: {}", response);
}
