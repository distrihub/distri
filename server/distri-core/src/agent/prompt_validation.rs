use comfy_table::{Attribute, Cell, Color, Table};
/// Prompt validation utilities for agent templates
use distri_types::StandardDefinition;

#[derive(Debug, Clone, PartialEq)]
pub enum Criticality {
    Critical,
    Medium,
    Low,
}

impl Criticality {
    pub fn as_str(&self) -> &str {
        match self {
            Criticality::Critical => "CRITICAL",
            Criticality::Medium => "MEDIUM",
            Criticality::Low => "LOW",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Criticality::Critical => Color::Red,
            Criticality::Medium => Color::Yellow,
            Criticality::Low => Color::Blue,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ValidationIssue {
    pub category: String,
    pub missing_items: Vec<String>,
    pub description: String,
    pub criticality: Criticality,
}

/// Validate custom prompt for missing essential partials and variables
pub fn validate_custom_prompt(
    template: &str,
    _agent_def: &StandardDefinition,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // Define criticality levels for different partials
    let partial_criticality = [
        ("core_instructions", Criticality::Medium),
        ("communication", Criticality::Low),
        ("tools_xml", Criticality::Medium),
        ("tools_json", Criticality::Medium),
        ("reasoning", Criticality::Medium),
    ];

    // Define criticality levels for variables that templates should include
    // Note: System variables like tool_format, max_steps, etc. are always provided
    let variable_criticality = [
        ("task", Criticality::Critical),
        ("available_tools", Criticality::Critical),
        ("scratchpad", Criticality::Medium),
    ];

    // Check for missing essential partials
    let missing_partials: Vec<(&str, Criticality)> = partial_criticality
        .iter()
        .filter(|(partial, _)| !template.contains(&format!("{{{{> {}}}}}", partial)))
        .map(|(partial, crit)| (*partial, crit.clone()))
        .collect();

    for (partial, criticality) in missing_partials {
        issues.push(ValidationIssue {
            category: "Missing Partial".to_string(),
            missing_items: vec![partial.to_string()],
            description: match criticality {
                Criticality::Critical => "Essential for agent functionality".to_string(),
                Criticality::Medium => "Important for proper agent behavior".to_string(),
                Criticality::Low => "Recommended for better user experience".to_string(),
            },
            criticality,
        });
    }

    // Check for missing essential variables
    let missing_variables: Vec<(&str, Criticality)> = variable_criticality
        .iter()
        .filter(|(variable, _)| !template.contains(&format!("{{{{{}}}}}", variable)))
        .map(|(variable, crit)| (*variable, crit.clone()))
        .collect();

    for (variable, criticality) in missing_variables {
        issues.push(ValidationIssue {
            category: "Missing Variable".to_string(),
            missing_items: vec![variable.to_string()],
            description: match criticality {
                Criticality::Critical => "Required for agent execution".to_string(),
                Criticality::Medium => "Important for agent context".to_string(),
                Criticality::Low => "Optional but recommended".to_string(),
            },
            criticality,
        });
    }

    // Optional: Check for best practices with paired system variables
    // (These are system-provided variables, so this is just a suggestion)
    if template.contains("{{max_steps}}") && !template.contains("{{remaining_steps}}") {
        issues.push(ValidationIssue {
            category: "Best Practice".to_string(),
            missing_items: vec!["remaining_steps".to_string()],
            description:
                "Consider using remaining_steps alongside max_steps for better step tracking"
                    .to_string(),
            criticality: Criticality::Low,
        });
    }

    // Note: System variables like tool_format, current_steps, etc. are always provided during rendering

    issues
}

/// Create a formatted table display of validation issues
pub fn format_validation_table(agent_name: &str, issues: &[ValidationIssue]) -> String {
    if issues.is_empty() {
        return format!(
            "✅ Agent '{}' prompt validation passed - no issues found",
            agent_name
        );
    }

    let mut table = Table::new();
    table.set_header(vec![
        Cell::new("Category").add_attribute(Attribute::Bold),
        Cell::new("Missing Item").add_attribute(Attribute::Bold),
        Cell::new("Criticality").add_attribute(Attribute::Bold),
        Cell::new("Description").add_attribute(Attribute::Bold),
    ]);

    // Sort issues by criticality (Critical first, then Medium, then Low)
    let mut sorted_issues = issues.to_vec();
    sorted_issues.sort_by(|a, b| match (&a.criticality, &b.criticality) {
        (Criticality::Critical, Criticality::Critical) => std::cmp::Ordering::Equal,
        (Criticality::Critical, _) => std::cmp::Ordering::Less,
        (_, Criticality::Critical) => std::cmp::Ordering::Greater,
        (Criticality::Medium, Criticality::Medium) => std::cmp::Ordering::Equal,
        (Criticality::Medium, Criticality::Low) => std::cmp::Ordering::Less,
        (Criticality::Low, Criticality::Medium) => std::cmp::Ordering::Greater,
        (Criticality::Low, Criticality::Low) => std::cmp::Ordering::Equal,
    });

    for issue in &sorted_issues {
        let criticality_cell = Cell::new(issue.criticality.as_str())
            .fg(issue.criticality.color())
            .add_attribute(Attribute::Bold);

        table.add_row(vec![
            Cell::new(&issue.category),
            Cell::new(&issue.missing_items.join(", ")),
            criticality_cell,
            Cell::new(&issue.description),
        ]);
    }

    let critical_count = issues
        .iter()
        .filter(|i| i.criticality == Criticality::Critical)
        .count();
    let medium_count = issues
        .iter()
        .filter(|i| i.criticality == Criticality::Medium)
        .count();
    let low_count = issues
        .iter()
        .filter(|i| i.criticality == Criticality::Low)
        .count();

    format!(
        "⚠️  Agent '{}' prompt validation found {} issue(s): {} critical, {} medium, {} low\n\n{}",
        agent_name,
        issues.len(),
        critical_count,
        medium_count,
        low_count,
        table
    )
}

/// Validate agent prompt based on its strategy (returns detailed issues)
pub fn validate_agent_prompt(agent_def: &StandardDefinition) -> Vec<ValidationIssue> {
    match agent_def
        .append_default_instructions
        .as_ref()
        .unwrap_or(&true)
    {
        false => validate_custom_prompt(&agent_def.instructions, agent_def),
        true => {
            // For append strategy, no specific validation needed as it uses the standard template
            Vec::new()
        }
    }
}
