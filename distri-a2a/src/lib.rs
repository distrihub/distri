mod a2a_types;
pub use a2a_types::*;

#[cfg(test)]
mod tests {
    use crate::AgentCard;
    use schemars::schema_for;
    use serde_json;
    use std::fs;

    #[test]
    fn validate_agent_card_schema() {
        let schema = schema_for!(AgentCard);
        let schema_json = serde_json::to_string_pretty(&schema).unwrap();

        // Optional: save the generated schema to a file for inspection
        // fs::write("schema.json", &schema_json).unwrap();

        let spec_schema_str = fs::read_to_string("a2a.json").unwrap();
        let spec_schema: serde_json::Value = serde_json::from_str(&spec_schema_str).unwrap();

        let generated_schema: serde_json::Value = serde_json::from_str(&schema_json).unwrap();

        assert_eq!(
            generated_schema["definitions"]["AgentCard"],
            spec_schema["components"]["schemas"]["AgentCard"]
        );
    }
}
