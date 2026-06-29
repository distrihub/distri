//! Channel-facing rendering types. A `WorkflowAgent` bound to a bot
//! uses these for slash command declarations (now on
//! [`crate::WorkflowTrigger::Slash`]) and reply rendering.
//! See `docs/specs/workflow-channel-commands.md`.
//!
//! Note: the old `ChannelTrigger` enum (`Slash`/`Callback`/`Message`)
//! is gone — it's superseded by [`crate::WorkflowTrigger`] which folds
//! channel triggers together with manual / schedule / webhook / event
//! / tool triggers into one enum.

use crate::channels::ChannelProvider;
use crate::configuration::AgentConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Built-in slash commands every distri agent supports without declaring them.
///
/// `Custom(String)` is the extension point: surfaces that ship their own
/// commands wrap them as `SystemCommand::Custom("workspace")`, and the
/// resolver (or downstream router) matches on the string. distri-cloud
/// uses this to layer its `CloudCommand` set on top — see
/// `distri-cloud/distri-gateway/src/command_router.rs::CloudCommand`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "name")]
pub enum SystemCommand {
    /// `/compact` — runs the manual compactor on the current task.
    Compact,
    /// `/usage` — render the current `ContextBudget` breakdown.
    Usage,
    /// `/clear` — start a new thread, keep the same agent.
    Clear,
    /// `/help` — list every command surfaced by the current agent.
    Help,
    /// Extension slot for surface-specific commands. The string is the bare
    /// command name *without* the leading slash (e.g. `"workspace"`).
    Custom(String),
}

impl SystemCommand {
    /// Match against the bare slash-command name (with leading `/`).
    /// Returns `None` for names that don't correspond to a built-in.
    /// Use `from_name_with_custom` if you want unknown names to fall
    /// through into the `Custom` variant.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "/compact" => Some(Self::Compact),
            "/usage" => Some(Self::Usage),
            "/clear" => Some(Self::Clear),
            "/help" => Some(Self::Help),
            _ => None,
        }
    }

    /// Like `from_name`, but unknown names produce a `Custom(stripped)`
    /// rather than `None`. Useful for higher-level routers that layer
    /// surface-specific commands on top of the built-ins.
    pub fn from_name_with_custom(name: &str) -> Self {
        Self::from_name(name).unwrap_or_else(|| {
            let stripped = name.strip_prefix('/').unwrap_or(name).to_string();
            Self::Custom(stripped)
        })
    }

    /// The canonical slash name including the leading `/`.
    pub fn name(&self) -> String {
        match self {
            Self::Compact => "/compact".to_string(),
            Self::Usage => "/usage".to_string(),
            Self::Clear => "/clear".to_string(),
            Self::Help => "/help".to_string(),
            Self::Custom(n) => {
                if n.starts_with('/') {
                    n.clone()
                } else {
                    format!("/{n}")
                }
            }
        }
    }

    /// One-line description used by `/help` and channel menus. `Custom`
    /// carries no description — surfaces that own a `Custom` should
    /// provide their own description elsewhere.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Compact => "Summarize prior turns to free context",
            Self::Usage => "Show current context usage",
            Self::Clear => "Start a new thread, keep the same agent",
            Self::Help => "List available commands",
            Self::Custom(_) => "",
        }
    }

    /// Iterate the four built-in (non-Custom) commands.
    pub fn builtins() -> [SystemCommand; 4] {
        [Self::Compact, Self::Usage, Self::Clear, Self::Help]
    }
}

/// A slash command declared by a `StandardAgent` (`StandardDefinition.commands`).
///
/// This is the standard-agent counterpart of a `WorkflowAgent`'s entry-point
/// `WorkflowTrigger::Slash`: both surface as channel slash commands, compiled
/// into the gateway's single `CommandRouter`. A standard-agent command is a
/// **preset prompt** — invoking it sends `prompt` to the agent as the user
/// message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SlashCommand {
    /// Slash name, e.g. "/summary". Leading "/" required.
    pub name: String,
    /// One-line description for `/help` and the channel command menu.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Alternate names that resolve to this command.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// Restrict to these providers; empty = all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<ChannelProvider>,
    /// Preset prompt sent to the agent when this command is invoked. Any text
    /// the user typed after the command is appended.
    pub prompt: String,
}

/// Convert a `SystemCommand` into the `SlashCommand` shape that channel
/// menus / help renderers consume. `Custom(_)` produces a placeholder with
/// an empty description — surfaces that own a Custom command should attach
/// a real description by passing their own `SlashCommand` to
/// `resolve_commands_with`.
pub fn system_command_as_slash(cmd: &SystemCommand) -> SlashCommand {
    SlashCommand {
        name: cmd.name(),
        description: cmd.description().to_string(),
        aliases: vec![],
        channels: vec![],
        prompt: String::new(),
    }
}

/// Built-in slash commands every agent gets without having to declare them.
/// Resolved **client-side** without going through the agent loop:
///
/// - `/compact` → calls `POST /v1/tasks/{task_id}/compact`
/// - `/usage`   → reads `ContextBudget` from local state, renders inline
/// - `/clear`   → opens a new thread with the same agent
/// - `/help`    → renders the resolved commands list
///
/// Channel surfaces (Slack/Telegram) get them via the gateway's
/// `CommandRouter`, which short-circuits these names before dispatching to
/// the agent. A per-agent command with the same name shadows the system one.
pub fn system_commands() -> Vec<SlashCommand> {
    SystemCommand::builtins()
        .iter()
        .map(system_command_as_slash)
        .collect()
}

/// Resolve the full slash-command surface for an agent: system commands
/// first, then agent-declared ones. A duplicate name (by exact name) drops
/// the system command — explicit override wins.
pub fn resolve_commands(agent: &AgentConfig) -> Vec<SlashCommand> {
    resolve_commands_with(agent, &[])
}

/// Like `resolve_commands`, but takes an additional set of `extra` commands
/// that get folded in alongside the built-ins. Surfaces that ship their own
/// commands (e.g. distri-cloud's `CloudCommand` set) use this to layer
/// them in once and benefit from the same shadow-by-name rule.
///
/// Precedence (highest wins on duplicate `name`): agent-declared >
/// extras > built-in system commands.
pub fn resolve_commands_with(agent: &AgentConfig, extra: &[SlashCommand]) -> Vec<SlashCommand> {
    let mut out = system_commands();

    // Layer extras on top of built-ins.
    if !extra.is_empty() {
        let extra_names: std::collections::HashSet<String> =
            extra.iter().map(|c| c.name.clone()).collect();
        out.retain(|c| !extra_names.contains(&c.name));
        out.extend(extra.iter().cloned());
    }

    let agent_cmds: Vec<SlashCommand> = match agent {
        AgentConfig::StandardAgent(d) => d.commands.clone(),
        // Workflow agents don't declare `SlashCommand`s — their commands live
        // on `WorkflowTrigger::Slash` entry points. We only fold in system
        // commands here so they get `/compact`, `/usage`, etc. for free.
        AgentConfig::WorkflowAgent(_) => Vec::new(),
    };

    if !agent_cmds.is_empty() {
        let declared: std::collections::HashSet<String> =
            agent_cmds.iter().map(|c| c.name.clone()).collect();
        out.retain(|c| !declared.contains(&c.name));
        out.extend(agent_cmds);
    }
    out
}

/// Whether `name` is one of the four built-in (non-Custom) system commands.
pub fn is_system_command(name: &str) -> bool {
    SystemCommand::from_name(name).is_some()
}

/// Author-facing button template inside a `StepKind::Reply`. Label/url/
/// callback_data may contain `{...}` interpolation (resolved by the
/// Reply step executor against workflow context, and `{item.*}` when
/// used as a `button_template`).
///
/// Resolved into [`ChannelButton`] by the Reply step executor once
/// interpolation is applied. Both types are intentionally kept separate:
/// this one is the template that lives in the workflow definition; the
/// other is the concrete value that crosses the executor → gateway boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplyButtonSpec {
    Url {
        label: String,
        url: String,
    },
    WebApp {
        label: String,
        url: String,
    },
    Callback {
        label: String,
        callback_data: String,
    },
}

/// A fully-resolved button (no interpolation left). Crosses the
/// workflow-executor → gateway boundary inside `AgentEventType::ChannelReply`.
///
/// The resolved counterpart of [`ReplyButtonSpec`] — no `{...}` placeholders
/// remain. Produced by the Reply step executor after applying interpolation
/// against workflow context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelButton {
    Url {
        label: String,
        url: String,
    },
    WebApp {
        label: String,
        url: String,
    },
    Callback {
        label: String,
        callback_data: String,
    },
}

/// A fully-resolved channel reply emitted by a `StepKind::Reply` step.
/// `buttons` is rows of buttons (outer = rows top-to-bottom).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelReply {
    pub text: String,
    #[serde(default)]
    pub buttons: Vec<Vec<ChannelButton>>,
}

/// Channel chrome for a workflow-agent bot (presentation only — per-
/// command behavior lives in entry points). All fields optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ChannelBindings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram: Option<TelegramBinding>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TelegramBinding {
    /// In-chat persistent menu button.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_button: Option<MenuButton>,
    /// Base URL prepended to relative WebApp button URLs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_app_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct MenuButton {
    pub label: String,
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_button_spec_kinds_round_trip() {
        for json in [
            serde_json::json!({"kind":"url","label":"Docs","url":"https://d.dev"}),
            serde_json::json!({"kind":"web_app","label":"Open","url":"https://a.app"}),
            serde_json::json!({"kind":"callback","label":"Pick","callback_data":"wf:x"}),
        ] {
            let b: ReplyButtonSpec = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(serde_json::to_value(&b).unwrap(), json);
        }
    }

    #[test]
    fn channel_bindings_defaults_are_none() {
        let b: ChannelBindings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(b.telegram.is_none());
    }

    #[test]
    fn system_command_round_trips_builtins() {
        for cmd in SystemCommand::builtins() {
            let resolved = SystemCommand::from_name(&cmd.name()).unwrap();
            assert_eq!(resolved, cmd);
            assert!(!cmd.description().is_empty());
        }
    }

    #[test]
    fn system_command_falls_through_to_custom() {
        let cmd = SystemCommand::from_name_with_custom("/workspace");
        assert_eq!(cmd, SystemCommand::Custom("workspace".to_string()));
        // Built-in name still resolves to the built-in variant.
        assert_eq!(
            SystemCommand::from_name_with_custom("/compact"),
            SystemCommand::Compact
        );
        // Round-trips the name back with the leading slash.
        assert_eq!(SystemCommand::Custom("ws".into()).name(), "/ws");
    }

    #[test]
    fn resolve_commands_with_layers_extras() {
        let cfg: AgentConfig = serde_json::from_value(serde_json::json!({
            "agent_type": "standard_agent",
            "name": "s",
            "description": "d",
        }))
        .unwrap();
        let extra = vec![SlashCommand {
            name: "/workspace".into(),
            description: "Open workspace switcher".into(),
            aliases: vec![],
            channels: vec![],
            prompt: String::new(),
        }];
        let resolved = resolve_commands_with(&cfg, &extra);
        let names: Vec<&str> = resolved.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"/compact"));
        assert!(names.contains(&"/workspace"));
    }

    #[test]
    fn resolve_commands_with_extras_shadow_builtins() {
        // An extra `/help` overrides the built-in description.
        let cfg: AgentConfig = serde_json::from_value(serde_json::json!({
            "agent_type": "standard_agent",
            "name": "s",
            "description": "d",
        }))
        .unwrap();
        let extra = vec![SlashCommand {
            name: "/help".into(),
            description: "Cloud help".into(),
            aliases: vec![],
            channels: vec![],
            prompt: String::new(),
        }];
        let resolved = resolve_commands_with(&cfg, &extra);
        let help: Vec<&SlashCommand> = resolved.iter().filter(|c| c.name == "/help").collect();
        assert_eq!(help.len(), 1);
        assert_eq!(help[0].description, "Cloud help");
    }

    #[test]
    fn system_commands_present_for_standard_agent_without_commands() {
        let cfg: AgentConfig = serde_json::from_value(serde_json::json!({
            "agent_type": "standard_agent",
            "name": "s",
            "description": "d",
        }))
        .unwrap();
        let resolved = resolve_commands(&cfg);
        let names: Vec<&str> = resolved.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"/compact"));
        assert!(names.contains(&"/usage"));
        assert!(names.contains(&"/clear"));
        assert!(names.contains(&"/help"));
    }

    #[test]
    fn agent_command_shadows_system_command() {
        let cfg: AgentConfig = serde_json::from_value(serde_json::json!({
            "agent_type": "standard_agent",
            "name": "s",
            "description": "d",
            "commands": [
                {"name":"/compact","description":"override","prompt":"do it"}
            ]
        }))
        .unwrap();
        let resolved = resolve_commands(&cfg);
        let compact: Vec<&SlashCommand> =
            resolved.iter().filter(|c| c.name == "/compact").collect();
        assert_eq!(compact.len(), 1);
        assert_eq!(compact[0].prompt, "do it");
    }

    #[test]
    fn workflow_agent_still_gets_system_commands() {
        let cfg: AgentConfig = serde_json::from_value(serde_json::json!({
            "agent_type": "workflow_agent",
            "name": "w",
            "description": "d",
            "definition": {}
        }))
        .unwrap();
        let resolved = resolve_commands(&cfg);
        assert!(resolved.iter().any(|c| c.name == "/compact"));
    }

    #[test]
    fn telegram_menu_button_round_trips() {
        let json = serde_json::json!({
            "telegram": {
                "menu_button": {"label":"Open","url":"https://a.app/learn"},
                "web_app_base": "https://a.app"
            }
        });
        let b: ChannelBindings = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(
            b.telegram
                .as_ref()
                .unwrap()
                .menu_button
                .as_ref()
                .unwrap()
                .label,
            "Open"
        );
        assert_eq!(serde_json::to_value(&b).unwrap(), json);
    }

    #[test]
    fn channel_reply_holds_resolved_buttons() {
        let r = ChannelReply {
            text: "Your classes:".into(),
            buttons: vec![vec![ChannelButton::Callback {
                label: "Math".into(),
                callback_data: "wf:open_class:m1".into(),
            }]],
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: ChannelReply = serde_json::from_value(v).unwrap();
        assert_eq!(back.text, "Your classes:");
        assert_eq!(back.buttons[0].len(), 1);
    }
}
