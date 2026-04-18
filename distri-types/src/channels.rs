//! Shared channel + bot types. Consumed by `distri-cloud` (server, stores,
//! handlers) and `distri-gateway` (webhook adapters) so the two halves speak
//! the same vocabulary.
//!
//! **Model**
//!
//! - A [`Bot`] is a configured messaging-platform bot (Telegram bot token,
//!   WhatsApp business number, Discord app). One row per bot. Holds workspace,
//!   agent, trigger mode, credentials.
//! - A [`Channel`] is a single conversation: a Telegram DM or group, a
//!   WhatsApp chat, a Discord channel. One row per `(bot_id, chat_id)`. Holds
//!   thread state and verification status.
//! - [`TriggerMode`] controls whether the bot responds to all messages or only
//!   those that mention it.
//! - [`PlatformAuthScope`] distinguishes platforms with workspace-level auth
//!   (Slack, Discord) from open platforms (Telegram, WhatsApp).
//! - [`AuthenticatedChannelUser`] is the type-level proof that the auth gate
//!   was crossed for a particular `(Channel, PlatformUser)` pair.

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

// ── Trigger mode ──────────────────────────────────────────────────────────

/// Whether the bot responds to all messages in a chat or only those that
/// mention it by username. Relevant for group chats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TriggerMode {
    All,
    OnMention,
}

impl Default for TriggerMode {
    fn default() -> Self {
        TriggerMode::All
    }
}

impl TriggerMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            TriggerMode::All => "all",
            TriggerMode::OnMention => "on_mention",
        }
    }
}

impl fmt::Display for TriggerMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for TriggerMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "all" => Ok(Self::All),
            "on_mention" => Ok(Self::OnMention),
            other => Err(format!("unknown trigger mode: {other}")),
        }
    }
}

// ── Platform auth scope ───────────────────────────────────────────────────

/// Whether a platform has built-in workspace-level authentication
/// (Slack OAuth installs, Discord guild memberships) or is open-access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformAuthScope {
    /// Anyone with the bot's handle can message it (Telegram, WhatsApp).
    Open,
    /// The bot is installed into a workspace; only members can interact
    /// (Slack, Discord).
    WorkspaceScoped,
}

impl ChannelProvider {
    pub fn platform_auth_scope(&self) -> PlatformAuthScope {
        match self {
            ChannelProvider::Telegram | ChannelProvider::Whatsapp => PlatformAuthScope::Open,
            ChannelProvider::Slack | ChannelProvider::Discord => PlatformAuthScope::WorkspaceScoped,
        }
    }
}

// ── Bot ───────────────────────────────────────────────────────────────────

/// A configured bot on a messaging platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bot {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub created_by_user_id: Uuid,
    pub provider: ChannelProvider,
    /// Platform handle — Telegram `@username`, WhatsApp `phone_number_id`
    /// (Meta's opaque internal ID, NOT the user-facing phone number),
    /// Discord bot client id, etc. Used by webhook/send paths at runtime.
    pub bot_username: Option<String>,
    /// User-visible phone number for WhatsApp bots (e.g. `+14028760395`).
    /// Returned by Meta's `phone_numbers` API at create time; persisted so
    /// the UI can render `wa.me/` deep links and a real number instead of
    /// the opaque `phone_number_id` in `bot_username`. Empty string for
    /// non-WhatsApp providers and for legacy rows where it's unknown.
    #[serde(default)]
    pub display_phone_number: String,
    /// Bot credential token.
    pub bot_token: Option<String>,
    /// Per-bot HMAC for inbound webhook validation.
    pub webhook_secret: Option<String>,
    /// Which agent handles messages routed through this bot.
    pub agent_id: String,
    pub trigger_mode: TriggerMode,
    pub active: bool,
    /// True iff this row is a platform-shared system bot
    /// (`workspace_id == Uuid::nil()`). Computed at read time from the
    /// workspace id; not a persisted column. Clients use this to render
    /// system bots with a `System` pill and lock down delete/edit actions.
    #[serde(default)]
    pub is_system: bool,
}

/// Payload for creating a new bot row.
#[derive(Debug, Clone)]
pub struct NewBot {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub created_by_user_id: Uuid,
    pub provider: ChannelProvider,
    pub bot_username: Option<String>,
    /// See [`Bot::display_phone_number`]. Empty for non-WhatsApp providers.
    pub display_phone_number: String,
    pub bot_token: Option<String>,
    pub webhook_secret: Option<String>,
    pub agent_id: String,
    pub trigger_mode: TriggerMode,
    pub active: bool,
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
    /// Whether this channel has been verified (a pairing handshake has
    /// completed for at least one connection).
    pub verified: bool,
    /// The `channel_identities.id` of the user who first opened this channel.
    pub created_by_identity_id: Option<Uuid>,
    #[serde(default)]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Payload for creating a new channel row.
#[derive(Debug, Clone)]
pub struct NewChannel {
    pub bot_id: Uuid,
    pub chat_id: String,
    pub chat_type: ChatType,
    pub thread_id: Option<String>,
    pub verbose: bool,
    /// Defaults to `false`; set to `true` after a pairing handshake.
    pub verified: bool,
    pub created_by_identity_id: Option<Uuid>,
}

// ── Bot connection join ────────────────────────────────────────────────────

/// A connection wired up to a bot. One bot can have multiple connections;
/// the `position` field controls which one is tried first.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConnection {
    pub bot_id: Uuid,
    pub connection_id: Uuid,
    pub requires_setup: bool,
    pub position: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ── Channel verification ───────────────────────────────────────────────────

/// Records a completed pairing handshake: a specific user on a specific
/// channel has proven their identity via a particular connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelVerification {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub connection_id: Uuid,
    pub verified_by_user_id: Uuid,
    pub external_user_id: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: Option<chrono::DateTime<chrono::Utc>>,
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
/// granted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthProof {
    /// Platform verified by default — no distri gate needed (e.g. Slack OAuth install).
    PlatformVerified,
    /// Open platform (Telegram/WhatsApp) with no connection gate — anyone can use.
    Open,
    /// Access granted because the user passed the gate for this connection.
    GatedBy { connection_id: Uuid },
}

/// Outcome of running the channel-auth resolver against an inbound message.
/// The gateway webhook handler turns each variant into a concrete reply.
#[derive(Debug, Clone)]
pub enum ResolveOutcome {
    Authenticated(AuthenticatedChannelUser),
    /// The channel/user needs to complete a verification flow.
    NeedsVerification { url: String, gate_kind: GateKind },
    /// No path exists for this user to access the bot.
    Denied { reason: String },
    /// The message should be silently rejected (e.g. unknown update type).
    Rejected,
}

/// Describes what kind of gate needs to be passed for verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateKind {
    /// Gate is the distri-native account link flow.
    DistriNative,
    /// Gate is an external connection with the given id.
    External { connection_id: Uuid },
}
