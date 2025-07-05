use crate::types::AgentDefinition;

pub fn validate_agent_definition(definition: &AgentDefinition) -> anyhow::Result<()> {
    if definition.name.is_empty() {
        return Err(anyhow::anyhow!("name must be set"));
    }
    if definition.plan.is_some() {
        let plan = definition.plan.as_ref().unwrap();
        if plan.interval <= 0 {
            return Err(anyhow::anyhow!("plan.interval must be greater than 0"));
        }
        if plan.max_iterations.is_some() && plan.max_iterations.unwrap() <= 0 {
            return Err(anyhow::anyhow!(
                "plan.max_iterations must be greater than 0"
            ));
        }
    }

    Ok(())
}
