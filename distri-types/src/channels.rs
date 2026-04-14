//! Shared channel + bot types. Consumed by `distri-cloud` (server, stores,
//! handlers) and `distri-gateway` (webhook adapters) so the two halves speak
//! the same vocabulary.
//!
//! **Model**
//!
//! - A [`Bot`] is a configured messaging-platform bot (Telegram bot token,
//!   WhatsApp business number, Discord app). One row per bot. Holds workspace,
//!   agent, auth scope, connection binding, credentials.
//! - A [`Channel`] is a single conversation: a Telegram DM or group, a
//!   WhatsApp chat, a Discord channel. One row per `(bot_id, chat_id)`. Holds
//!   thread state only — everything else comes from the parent [`Bot`].
//! - [`AuthScope`] is the 3-variant gate: Public, Workspace, User.
//! - [`AuthenticatedChannelUser`] is the type-level proof that the auth gate
//!   was crossed for a particular `(Channel, PlatformUser)` pair. Every
//!   downstream function that dispatches to an agent must take one.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Providers ─────────────────────────────────────────────────────────────

/// The messaging platform a bot / channel lives on.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ChannelProvider {
    Telegram,
    Whatsapp,
    Discord,
    Slack,
}

impl fmt::Display for ChannelProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Telegram => "telegram",
            Self::Whatsapp => "whatsapp",
            Self::Discord => "discord",
            Self::Slack => "slack",
        })
    }
}

impl std::str::FromStr for ChannelProvider {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "telegram" => Ok(Self::Telegram),
            "whatsapp" => Ok(Self::Whatsapp),
            "discord" => Ok(Self::Discord),
            "slack" => Ok(Self::Slack),
            other => Err(format!("unknown channel provider: {other}")),
        }
    }
}

/// Telegram-style chat types. Generalised across platforms — DMs are
/// `Private`, groups are `Group`, channels are `Channel`. WhatsApp / Discord
/// adapters map their own types onto these.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ChatType {
    Private,
    Group,
    Supergroup,
    Channel,
}

impl fmt::Display for ChatType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Private => "private",
            Self::Group => "group",
            Self::Supergroup => "supergroup",
            Self::Channel => "channel",
        })
    }
}

impl std::str::FromStr for ChatType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "private" => Ok(Self::Private),
            "group" => Ok(Self::Group),
            "supergroup" => Ok(Self::Supergroup),
            "channel" => Ok(Self::Channel),
            other => Err(format!("unknown chat type: {other}")),
        }
    }
}

// ── Auth scope (3-variant, no coarse shim) ────────────────────────────────

/// Who is allowed to actually use a bot's channels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AuthScope {
    /// Anyone. Global identity is inline-created on first contact.
    Public,
    /// Platform members of the bot's workspace.
    Workspace,
    /// External users configured per-connection. First contact returns a
    /// configure URL that stores their values as user-scope secrets.
    User,
}

impl fmt::Display for AuthScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Public => "public",
            Self::Workspace => "workspace",
            Self::User => "user",
        })
    }
}

impl std::str::FromStr for AuthScope {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "public" => Ok(Self::Public),
            "workspace" => Ok(Self::Workspace),
            "user" => Ok(Self::User),
            other => Err(format!("unknown auth scope: {other}")),
        }
    }
}

// ── Bot ───────────────────────────────────────────────────────────────────

/// A configured bot on a messaging platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bot {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub provider: ChannelProvider,
    /// Bot handle (`@testzippybot` on Telegram, phone number on WhatsApp, etc.).
    /// `None` for the platform-shared official bot which uses env credentials.
    pub bot_username: Option<String>,
    /// Bot credential. `None` for the platform-shared official bot.
    pub bot_token: Option<String>,
    /// Per-bot HMAC for inbound webhook validation.
    pub webhook_secret: Option<String>,
    /// Which agent handles messages routed through this bot.
    pub agent_id: String,
    pub auth_scope: AuthScope,
    /// Required when `auth_scope == User`. Unused otherwise.
    pub connection_id: Option<Uuid>,
    pub active: bool,
}

/// Payload for creating a new bot row.
#[derive(Debug, Clone)]
pub struct NewBot {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub provider: ChannelProvider,
    pub bot_username: Option<String>,
    pub bot_token: Option<String>,
    pub webhook_secret: Option<String>,
    pub agent_id: String,
    pub auth_scope: AuthScope,
    pub connection_id: Option<Uuid>,
}

// ── Channel (pure conversation row) ───────────────────────────────────────

/// A single conversation under a [`Bot`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: Uuid,
    pub bot_id: Uuid,
    /// Platform conversation id. Telegram: `message.chat.id` as string.
    /// WhatsApp: sender phone. Discord: channel id.
    pub chat_id: String,
    pub chat_type: ChatType,
    pub thread_id: Option<String>,
    pub verbose: bool,
    pub active: bool,
}

// ── Platform user (raw input from the webhook) ────────────────────────────

/// Raw actor identity extracted from an inbound webhook message.
/// On Telegram: derived from `message.from` (sender), *not* `message.chat`.
#[derive(Debug, Clone)]
pub struct PlatformUser {
    pub provider: ChannelProvider,
    /// Platform-specific user id. Telegram: `from.id` as string.
    pub platform_id: String,
    pub platform_username: Option<String>,
    pub platform_display_name: Option<String>,
}

// ── Channel identity (the global actor row) ───────────────────────────────

/// Cached mapping `(provider, platform_id) → users.id`. Created on first
/// contact with any bot and reused across every bot and channel.
#[derive(Debug, Clone)]
pub struct ChannelIdentity {
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider: ChannelProvider,
    pub platform_id: String,
    pub platform_username: Option<String>,
    pub platform_display_name: Option<String>,
}

// ── Auth gate output ──────────────────────────────────────────────────────

/// Type-level proof that the channel-auth gate has been crossed for a
/// `(Bot, Channel, PlatformUser)` triple. Downstream message handling
/// takes `&AuthenticatedChannelUser` by reference; you can't call the
/// agent without one.
#[derive(Debug, Clone)]
pub struct AuthenticatedChannelUser {
    /// Global `users.id` of the sender.
    pub user_id: Uuid,
    pub identity: ChannelIdentity,
    /// Snapshot of the channel this auth is valid for.
    pub channel_id: Uuid,
    pub bot_id: Uuid,
    pub workspace_id: Uuid,
    /// How they cleared the gate.
    pub auth: AuthProof,
}

/// Discriminator on [`AuthenticatedChannelUser`] explaining *why* access was
/// granted. Commands like `/stop` inspect this to do the right unbinding.
#[derive(Debug, Clone)]
pub enum AuthProof {
    /// Public bot — no extra check.
    Public,
    /// The user is a platform member of the bot's workspace.
    WorkspaceMember { role: String },
    /// The user has configured the bot's linked Custom connection.
    UserBinding {
        connection_id: Uuid,
        external_user_id: String,
    },
}

/// Outcome of running the channel-auth resolver against an inbound message.
/// The gateway webhook handler turns each variant into a concrete reply:
/// `Authenticated` → dispatch to the agent; `NeedsConfiguration` → reply with
/// the configure URL; `Denied` → send the reason text.
#[derive(Debug, Clone)]
pub enum ResolveOutcome {
    Authenticated(AuthenticatedChannelUser),
    /// Only produced for `AuthScope::User` before the sender has stored the
    /// connection's required field values in the secrets store.
    NeedsConfiguration {
        url: String,
    },
    /// No path exists for this user to access the bot (Workspace non-members
    /// or misconfigured rows).
    Denied {
        reason: String,
    },
}
