use crate::types::{LlmDefinition, Message, MessagePart, MessageRole, ModelSettings};

pub async fn create_initial_plan(
    message: &Message,
    tools_description: &str,
    model: &(dyn Fn(Vec<Message>) -> futures::future::BoxFuture<'static, anyhow::Result<String>>
          + Send
          + Sync),
) -> anyhow::Result<(String, String)> {
    // First get facts about the task
    let facts_messages = vec![
        Message {
            role: MessageRole::System,
            name: Some("facts".to_string()),
            parts: vec![MessagePart::Text(
                "Analyze the given task and list the key facts and requirements.".to_string(),
            )],
            ..Default::default()
        },
        message.clone(),
    ];

    let facts = model(facts_messages).await?;

    // Then create plan based on facts
    let plan_messages = vec![
        Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::System,
            name: Some("plan".to_string()),
            parts: vec![MessagePart::Text(format!(
                "Create a detailed plan to solve the task. Available tools:\n{}",
                tools_description
            ))],
            ..Default::default()
        },
        Message {
            role: MessageRole::User,
            name: Some("task".to_string()),
            parts: message.parts.clone(),
            ..Default::default()
        },
    ];

    let plan = model(plan_messages).await?;

    Ok((facts, plan))
}

pub async fn update_plan(
    message: &Message,
    tools_description: &str,
    previous_steps: &[Message],
    remaining_steps: usize,
    model: &(dyn Fn(Vec<Message>) -> futures::future::BoxFuture<'static, anyhow::Result<String>>
          + Send
          + Sync),
) -> anyhow::Result<(String, String)> {
    // Update facts based on previous steps
    let facts_update_messages = vec![Message {
        role: MessageRole::System,
        name: Some("facts_update".to_string()),
        parts: vec![MessagePart::Text(
            "Based on the execution history, update the list of known facts.".to_string(),
        )],
        ..Default::default()
    }];
    let mut all_messages = facts_update_messages;
    all_messages.extend_from_slice(previous_steps);

    let updated_facts = model(all_messages).await?;

    // Create updated plan
    let plan_update_messages = vec![Message {
        
        role: MessageRole::System,
        name: Some("plan_update".to_string()),
        parts: vec![
            MessagePart::Text(format!(
                    "Update the execution plan based on progress. You have {} steps remaining. Available tools:\n{}",
                    remaining_steps, tools_description)
        )],
        ..Default::default()
    }];
    let mut all_messages = plan_update_messages;
    all_messages.extend_from_slice(previous_steps);
    all_messages.push(Message {
        role: MessageRole::User,
        name: Some("task".to_string()),
        parts: vec![MessagePart::Text(format!(
            "Updated facts:\n{}\n\nProvide updated plan for completing the task: {}",
            updated_facts,
            message
                .parts
                .iter()
                .find(|p| matches!(p, MessagePart::Text(_)))
                .map(|p| {
                    if let MessagePart::Text(text) = p {
                        text.clone()
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default()
        ))],
        ..Default::default()
    });

    let updated_plan = model(all_messages).await?;

    Ok((updated_facts, updated_plan))
}

pub fn get_planning_definition(model_settings: ModelSettings) -> LlmDefinition {
    LlmDefinition {
        name: "planner".to_string(),
        system_prompt: Some(
            concat!(
                "You are a planning assistant that helps break down tasks into clear steps.\n",
                "Given a task and available tools, you will:\n",
                "1. First analyze and list key facts about the task\n",
                "2. Then create a step-by-step plan considering the available tools\n",
                "Be concise but thorough in your analysis."
            )
            .to_string(),
        ),
        history_size: None,
        model_settings: model_settings.clone(),
        ..Default::default()
    }
}
