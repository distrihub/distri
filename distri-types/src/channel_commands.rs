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
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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
