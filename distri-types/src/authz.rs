//! Typed authorization primitives shared between distri-server and distri-cloud.
//!
//! Spec: `distri-cloud/docs/superpowers/specs/2026-05-24-auth-system-design.md`
//! Plan: `distri-cloud/docs/superpowers/plans/2026-05-24-auth-system-implementation.md`
//!
//! The model is two-axis: [`AuthScope`] (Workspace | User | Public) + typed
//! [`Permissions`] (a map of [`ResourceKind`] to a bitset of [`Action`]s).
//!
//! Stores implement [`Authorize`] with a permissive default; distri-cloud's
//! PG stores override it with real checks. The HTTP layer is gated by a
//! [`RouteAuthTable`] that maps `(path, method)` to required permissions.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub use crate::connections::AuthScope;

// ---------------------------------------------------------------------------
// ResourceKind + Action
// ---------------------------------------------------------------------------

/// The kinds of resources distri's authorization model knows about.
///
/// Each handler / store call asks: "may this `AuthContext` perform this
/// `Action` on this `ResourceKind`?". The mapping from URL → required
/// `(ResourceKind, Action)` lives in [`RouteAuthTable`].
#[derive(
    Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum ResourceKind {
    /// Calling agents (a2a message/send, message/stream, tasks/*).
    Completion,
    Agent,
    Skill,
    Workflow,
    Note,
    Bot,
    Secret,
    /// Workspace-scoped OAuth / auth connections.
    ConnectionWorkspace,
    /// End-user-scoped OAuth / auth connections.
    ConnectionUser,
    /// Workspace itself: members, invitations, settings, billing.
    Workspace,
    /// Spans, traces, usage.
    Observability,
    /// The /mcp endpoint (proxies MCP traffic).
    Mcp,
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ResourceKind::Completion => "completion",
            ResourceKind::Agent => "agent",
            ResourceKind::Skill => "skill",
            ResourceKind::Workflow => "workflow",
            ResourceKind::Note => "note",
            ResourceKind::Bot => "bot",
            ResourceKind::Secret => "secret",
            ResourceKind::ConnectionWorkspace => "connection_workspace",
            ResourceKind::ConnectionUser => "connection_user",
            ResourceKind::Workspace => "workspace",
            ResourceKind::Observability => "observability",
            ResourceKind::Mcp => "mcp",
        };
        f.write_str(s)
    }
}

impl FromStr for ResourceKind {
    type Err = AuthError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "completion" => ResourceKind::Completion,
            "agent" => ResourceKind::Agent,
            "skill" => ResourceKind::Skill,
            "workflow" => ResourceKind::Workflow,
            "note" => ResourceKind::Note,
            "bot" => ResourceKind::Bot,
            "secret" => ResourceKind::Secret,
            "connection_workspace" => ResourceKind::ConnectionWorkspace,
            "connection_user" => ResourceKind::ConnectionUser,
            "workspace" => ResourceKind::Workspace,
            "observability" => ResourceKind::Observability,
            "mcp" => ResourceKind::Mcp,
            _ => {
                return Err(AuthError::InvalidResourceKind(s.into()));
            }
        })
    }
}

/// A single action that can be performed on a [`ResourceKind`].
///
/// Stored as a `u8` bitmask inside [`ActionSet`].
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Action {
    Read = 0b0001,
    Write = 0b0010,
    /// Create, delete, configure.
    Manage = 0b0100,
    /// Call (Completion only, today).
    Execute = 0b1000,
}

impl Action {
    pub const ALL: [Action; 4] = [
        Action::Read,
        Action::Write,
        Action::Manage,
        Action::Execute,
    ];

    pub fn bit(self) -> u8 {
        self as u8
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Action::Read => "read",
            Action::Write => "write",
            Action::Manage => "manage",
            Action::Execute => "execute",
        };
        f.write_str(s)
    }
}

/// Bitset of [`Action`]s. Serialized as a `u8` integer in JSON.
#[derive(
    Debug, Default, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct ActionSet(pub u8);

impl ActionSet {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub fn all() -> Self {
        Self::from(Action::ALL.as_slice())
    }

    pub fn contains(&self, action: Action) -> bool {
        self.0 & action.bit() != 0
    }

    pub fn insert(&mut self, action: Action) {
        self.0 |= action.bit();
    }

    pub fn remove(&mut self, action: Action) {
        self.0 &= !action.bit();
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// True iff every action in `self` is also in `other`.
    pub fn is_subset_of(self, other: Self) -> bool {
        (self.0 & !other.0) == 0
    }

    pub fn iter(self) -> impl Iterator<Item = Action> {
        Action::ALL.into_iter().filter(move |a| self.contains(*a))
    }
}

impl From<Action> for ActionSet {
    fn from(action: Action) -> Self {
        Self(action.bit())
    }
}

impl From<&[Action]> for ActionSet {
    fn from(actions: &[Action]) -> Self {
        let mut s = Self::empty();
        for a in actions {
            s.insert(*a);
        }
        s
    }
}

impl<const N: usize> From<[Action; N]> for ActionSet {
    fn from(actions: [Action; N]) -> Self {
        Self::from(actions.as_slice())
    }
}

impl fmt::Display for ActionSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        f.write_str("[")?;
        for a in self.iter() {
            if !first {
                f.write_str(",")?;
            }
            write!(f, "{}", a)?;
            first = false;
        }
        f.write_str("]")
    }
}

// ---------------------------------------------------------------------------
// Permissions
// ---------------------------------------------------------------------------

/// The full grant carried by a token: a sparse map from [`ResourceKind`] to
/// the set of [`Action`]s the holder may perform.
#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Permissions(pub HashMap<ResourceKind, ActionSet>);

impl Permissions {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Full permissions on every known resource. **Local-dev only.** Production
    /// code must construct explicit grants.
    pub fn all() -> Self {
        let mut p = Self::empty();
        let all_actions = ActionSet::all();
        for kind in [
            ResourceKind::Completion,
            ResourceKind::Agent,
            ResourceKind::Skill,
            ResourceKind::Workflow,
            ResourceKind::Note,
            ResourceKind::Bot,
            ResourceKind::Secret,
            ResourceKind::ConnectionWorkspace,
            ResourceKind::ConnectionUser,
            ResourceKind::Workspace,
            ResourceKind::Observability,
            ResourceKind::Mcp,
        ] {
            p.0.insert(kind, all_actions);
        }
        p
    }

    /// Permissions for the spec §6.2 `scope=user` default token:
    /// `{Completion: [Execute], ConnectionUser: [Read, Write]}`.
    pub fn default_for_user_scope() -> Self {
        let mut p = Self::empty();
        p.grant(ResourceKind::Completion, ActionSet::from(Action::Execute));
        p.grant(
            ResourceKind::ConnectionUser,
            ActionSet::from([Action::Read, Action::Write]),
        );
        p
    }

    /// Minimal permissions for outbound a2a tokens minted by
    /// `mint_session_token`. The remote endpoint can call agents and nothing
    /// else.
    pub fn default_for_completion() -> Self {
        let mut p = Self::empty();
        p.grant(ResourceKind::Completion, ActionSet::from(Action::Execute));
        p
    }

    /// Full member permissions for a workspace member. Used by the UI flow.
    pub fn default_for_workspace_member() -> Self {
        let mut p = Self::empty();
        let manage = ActionSet::from([Action::Read, Action::Write, Action::Manage]);
        p.grant(ResourceKind::Completion, ActionSet::from(Action::Execute));
        p.grant(ResourceKind::Agent, manage);
        p.grant(ResourceKind::Skill, manage);
        p.grant(ResourceKind::Workflow, manage);
        p.grant(ResourceKind::Note, manage);
        p.grant(ResourceKind::Bot, manage);
        p.grant(ResourceKind::Secret, manage);
        p.grant(ResourceKind::ConnectionWorkspace, manage);
        p.grant(ResourceKind::ConnectionUser, ActionSet::from(Action::Read));
        p.grant(
            ResourceKind::Workspace,
            ActionSet::from([Action::Read, Action::Manage]),
        );
        p.grant(ResourceKind::Observability, ActionSet::from(Action::Read));
        p.grant(ResourceKind::Mcp, ActionSet::from(Action::Execute));
        p
    }

    pub fn allows(&self, kind: ResourceKind, action: Action) -> bool {
        self.0.get(&kind).is_some_and(|s| s.contains(action))
    }

    pub fn grant(&mut self, kind: ResourceKind, actions: ActionSet) {
        let entry = self.0.entry(kind).or_insert(ActionSet::empty());
        *entry = entry.union(actions);
    }

    pub fn revoke(&mut self, kind: ResourceKind, actions: ActionSet) {
        if let Some(entry) = self.0.get_mut(&kind) {
            for a in actions.iter() {
                entry.remove(a);
            }
            if entry.is_empty() {
                self.0.remove(&kind);
            }
        }
    }

    /// Token attenuation: only keep actions the *other* set also allows.
    pub fn intersect(&self, other: &Self) -> Self {
        let mut out = Self::empty();
        for (kind, set) in &self.0 {
            if let Some(other_set) = other.0.get(kind) {
                let i = set.intersect(*other_set);
                if !i.is_empty() {
                    out.0.insert(*kind, i);
                }
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty() || self.0.values().all(|s| s.is_empty())
    }

    /// True iff every (kind, action) in self is also present in other.
    pub fn is_subset_of(&self, other: &Self) -> bool {
        for (kind, set) in &self.0 {
            let Some(other_set) = other.0.get(kind) else {
                if !set.is_empty() {
                    return false;
                }
                continue;
            };
            if !set.is_subset_of(*other_set) {
                return false;
            }
        }
        true
    }

    /// Flatten into the (kind, action) tuples expected by an actix-web-grants
    /// extractor or similar fixture-based testing.
    pub fn flatten(&self) -> Vec<(ResourceKind, Action)> {
        let mut out = Vec::new();
        for (kind, set) in &self.0 {
            for a in set.iter() {
                out.push((*kind, a));
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// AuthIdentity + AuthContext
// ---------------------------------------------------------------------------

/// How the caller authenticated.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    Jwt,
    ApiKey,
    Webhook,
    LocalDev,
}

/// JWT token kind classifier. Mirrors `cloud::TokenType`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum TokenType {
    #[default]
    Main,
    /// Short-lived JWT used for cross-deployment a2a hops.
    Short,
    Refresh,
}

/// Tier-derived call/rate limits (opaque to authz; carried along).
#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TierLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_calls: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<u32>,
    #[serde(default, flatten, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Identity carried by every authenticated request.
#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AuthIdentity {
    /// Internal Distri user id.
    pub user_id: Uuid,
    /// Present iff scope=Workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<Uuid>,
    /// Workspace role (e.g. `owner`, `admin`, `member`). Present iff
    /// scope=Workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_role: Option<String>,
    /// Present iff scope=User: the external user id from the bot/connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_user_external_id: Option<String>,
    /// Present iff scope=User: the connection provider id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_user_provider: Option<String>,
    /// Email of the user (optional; not all auth flows populate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Platform-admin flag (separate from workspace-level permissions).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_platform_admin: bool,
}

/// The full authorization context populated by middleware and consumed by
/// every handler / store.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AuthContext {
    pub scope: AuthScope,
    pub identity: AuthIdentity,
    pub permissions: Permissions,
    pub auth_method: AuthMethod,
    #[serde(default)]
    pub token_type: TokenType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<TierLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_client_id: Option<String>,
}

impl AuthContext {
    /// Construct a fully-permissioned local-dev context. **Only call this
    /// from tests or when the `DISTRI_LOCAL_DEV=1` env var is set.**
    pub fn local_dev() -> Self {
        Self {
            scope: AuthScope::Workspace,
            identity: AuthIdentity {
                user_id: Uuid::nil(),
                workspace_id: Some(Uuid::nil()),
                workspace_role: Some("owner".into()),
                email: Some("local-dev@distri.local".into()),
                is_platform_admin: true,
                ..Default::default()
            },
            permissions: Permissions::all(),
            auth_method: AuthMethod::LocalDev,
            token_type: TokenType::Main,
            limits: None,
            api_key_id: None,
            public_client_id: None,
        }
    }

    /// Construct an empty / unprivileged context — for tests of fail-closed
    /// behavior.
    pub fn empty_for_test() -> Self {
        Self {
            scope: AuthScope::Public,
            identity: AuthIdentity::default(),
            permissions: Permissions::empty(),
            auth_method: AuthMethod::LocalDev,
            token_type: TokenType::Main,
            limits: None,
            api_key_id: None,
            public_client_id: None,
        }
    }

    pub fn require(&self, kind: ResourceKind, action: Action) -> Result<(), AuthError> {
        if self.permissions.allows(kind, action) {
            Ok(())
        } else {
            Err(AuthError::MissingPermission { kind, action })
        }
    }

    pub fn require_scope(&self, scope: AuthScope) -> Result<(), AuthError> {
        if self.scope == scope {
            Ok(())
        } else {
            Err(AuthError::WrongScope {
                expected: scope,
                actual: self.scope,
            })
        }
    }

    pub fn require_workspace(&self, ws: Uuid) -> Result<(), AuthError> {
        match self.identity.workspace_id {
            Some(actual) if actual == ws => Ok(()),
            other => Err(AuthError::WrongWorkspace {
                expected: ws,
                actual: other,
            }),
        }
    }

    pub fn require_owner(&self, user: Uuid) -> Result<(), AuthError> {
        if self.identity.user_id == user {
            Ok(())
        } else {
            Err(AuthError::NotOwner)
        }
    }

    pub fn is_workspace_scope(&self) -> bool {
        matches!(self.scope, AuthScope::Workspace)
    }

    pub fn is_user_scope(&self) -> bool {
        matches!(self.scope, AuthScope::User)
    }
}

// ---------------------------------------------------------------------------
// AuthOp + Authorize
// ---------------------------------------------------------------------------

/// A logical operation a store/service is about to perform. Stores implement
/// [`Authorize::authorize`] to gate this.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum AuthOp {
    Read {
        kind: ResourceKind,
        id: Option<Uuid>,
    },
    Write {
        kind: ResourceKind,
        id: Option<Uuid>,
    },
    Manage {
        kind: ResourceKind,
        id: Option<Uuid>,
    },
    Execute {
        kind: ResourceKind,
        id: Option<Uuid>,
    },
}

impl AuthOp {
    pub fn kind(&self) -> ResourceKind {
        match self {
            AuthOp::Read { kind, .. }
            | AuthOp::Write { kind, .. }
            | AuthOp::Manage { kind, .. }
            | AuthOp::Execute { kind, .. } => *kind,
        }
    }

    pub fn action(&self) -> Action {
        match self {
            AuthOp::Read { .. } => Action::Read,
            AuthOp::Write { .. } => Action::Write,
            AuthOp::Manage { .. } => Action::Manage,
            AuthOp::Execute { .. } => Action::Execute,
        }
    }

    pub fn id(&self) -> Option<Uuid> {
        match self {
            AuthOp::Read { id, .. }
            | AuthOp::Write { id, .. }
            | AuthOp::Manage { id, .. }
            | AuthOp::Execute { id, .. } => *id,
        }
    }

    pub fn parts(&self) -> (ResourceKind, Action, Option<Uuid>) {
        (self.kind(), self.action(), self.id())
    }
}

/// Trait every store implements. The default returns `Ok(())` — distri-server
/// standalone (no cloud middleware) gets permissive behavior for free.
/// Distri-cloud PG stores **override** this with real per-resource enforcement.
#[async_trait::async_trait]
pub trait Authorize: Send + Sync {
    async fn authorize(
        &self,
        _ctx: &AuthContext,
        _op: &AuthOp,
    ) -> Result<(), AuthError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// AuthError
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Error, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum AuthError {
    #[error("unauthenticated")]
    Unauthenticated,

    #[error("no rule for route {method} {path}")]
    NoRuleForRoute { path: String, method: String },

    #[error("missing permission: {kind:?}:{action:?}")]
    MissingPermission {
        kind: ResourceKind,
        action: Action,
    },

    #[error("wrong scope: expected {expected:?}, got {actual:?}")]
    WrongScope {
        expected: AuthScope,
        actual: AuthScope,
    },

    #[error("wrong workspace: expected {expected}, got {actual:?}")]
    WrongWorkspace {
        expected: Uuid,
        actual: Option<Uuid>,
    },

    #[error("not the owner of this resource")]
    NotOwner,

    #[error("cloud auth registry not configured")]
    NotConfigured,

    #[error("invalid resource kind: {0}")]
    InvalidResourceKind(String),

    #[error("authorize store error: {0}")]
    StoreError(String),
}

impl AuthError {
    /// HTTP status the API layer should return for this error. Programmer
    /// errors (`NotConfigured`, `NoRuleForRoute`, `StoreError`) map to 500;
    /// auth-method failures map to 401; everything else 403.
    pub fn http_status(&self) -> u16 {
        match self {
            AuthError::Unauthenticated => 401,
            AuthError::NotConfigured
            | AuthError::NoRuleForRoute { .. }
            | AuthError::StoreError(_)
            | AuthError::InvalidResourceKind(_) => 500,
            _ => 403,
        }
    }
}

// ---------------------------------------------------------------------------
// RouteAuthTable
// ---------------------------------------------------------------------------

/// HTTP method, as carried in route rules. Kept here so distri-types doesn't
/// have to depend on actix.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
    /// Wildcard matching any HTTP method.
    Any,
}

impl HttpMethod {
    pub const ALL_VERBS: &'static [HttpMethod] = &[
        HttpMethod::Get,
        HttpMethod::Post,
        HttpMethod::Put,
        HttpMethod::Delete,
        HttpMethod::Patch,
    ];

    pub fn matches(self, other: HttpMethod) -> bool {
        self == HttpMethod::Any || other == HttpMethod::Any || self == other
    }
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Head => "HEAD",
            HttpMethod::Options => "OPTIONS",
            HttpMethod::Any => "*",
        };
        f.write_str(s)
    }
}

impl FromStr for HttpMethod {
    type Err = AuthError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_ascii_uppercase().as_str() {
            "GET" => HttpMethod::Get,
            "POST" => HttpMethod::Post,
            "PUT" => HttpMethod::Put,
            "DELETE" => HttpMethod::Delete,
            "PATCH" => HttpMethod::Patch,
            "HEAD" => HttpMethod::Head,
            "OPTIONS" => HttpMethod::Options,
            "*" | "ANY" => HttpMethod::Any,
            other => {
                return Err(AuthError::InvalidResourceKind(other.into()));
            }
        })
    }
}

/// What a route needs from a caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteRequirement {
    /// Public — no authentication required.
    Public,
    /// Caller must be authenticated, but no specific permission needed.
    /// Used for endpoints like `/v1/me` and `/v1/token` where the handler
    /// applies its own logic.
    Authenticated,
    /// Caller must hold this permission tuple.
    Permission {
        kind: ResourceKind,
        action: Action,
    },
    /// All-of: caller must hold every listed permission.
    AllOf(Vec<(ResourceKind, Action)>),
}

impl RouteRequirement {
    pub fn check(&self, perms: &Permissions) -> Result<(), AuthError> {
        match self {
            RouteRequirement::Public | RouteRequirement::Authenticated => Ok(()),
            RouteRequirement::Permission { kind, action } => {
                if perms.allows(*kind, *action) {
                    Ok(())
                } else {
                    Err(AuthError::MissingPermission {
                        kind: *kind,
                        action: *action,
                    })
                }
            }
            RouteRequirement::AllOf(needed) => {
                for (kind, action) in needed {
                    if !perms.allows(*kind, *action) {
                        return Err(AuthError::MissingPermission {
                            kind: *kind,
                            action: *action,
                        });
                    }
                }
                Ok(())
            }
        }
    }

    pub fn is_public(&self) -> bool {
        matches!(self, RouteRequirement::Public)
    }
}

/// One row in the route table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteRule {
    /// Path pattern. Supports `{name}` and `{name:.*}` placeholders, plus a
    /// trailing `*` wildcard.
    pub path_pattern: String,
    pub methods: Vec<HttpMethod>,
    pub requirement: RouteRequirement,
}

impl RouteRule {
    pub fn new(
        path_pattern: impl Into<String>,
        methods: impl IntoIterator<Item = HttpMethod>,
        requirement: RouteRequirement,
    ) -> Self {
        Self {
            path_pattern: path_pattern.into(),
            methods: methods.into_iter().collect(),
            requirement,
        }
    }

    pub fn public(path_pattern: impl Into<String>) -> Self {
        Self::new(
            path_pattern,
            [HttpMethod::Any],
            RouteRequirement::Public,
        )
    }

    pub fn authenticated(
        path_pattern: impl Into<String>,
        methods: impl IntoIterator<Item = HttpMethod>,
    ) -> Self {
        Self::new(path_pattern, methods, RouteRequirement::Authenticated)
    }

    pub fn perm(
        path_pattern: impl Into<String>,
        methods: impl IntoIterator<Item = HttpMethod>,
        kind: ResourceKind,
        action: Action,
    ) -> Self {
        Self::new(
            path_pattern,
            methods,
            RouteRequirement::Permission { kind, action },
        )
    }

    pub fn matches(&self, path: &str, method: HttpMethod) -> bool {
        self.methods
            .iter()
            .any(|m| m.matches(method))
            && pattern_matches(&self.path_pattern, path)
    }

    /// "Specificity" used to pick between overlapping rules. Higher = more
    /// specific. Exact paths score above wildcards; longer paths above
    /// shorter; rules with explicit methods above `Any`.
    pub fn specificity(&self) -> u32 {
        let mut score = 0u32;
        // Base on path: count non-wildcard segments + 1.
        let segments = self
            .path_pattern
            .split('/')
            .filter(|s| !s.is_empty())
            .count() as u32;
        score += segments * 100;
        if !self.path_pattern.ends_with('*') {
            score += 50;
        }
        if !self.path_pattern.contains('{') {
            score += 10;
        }
        if !self.methods.iter().any(|m| *m == HttpMethod::Any) {
            score += 5;
        }
        score
    }
}

/// Match `path` against a pattern that may contain `{name}` placeholders or a
/// trailing `*` wildcard.
fn pattern_matches(pattern: &str, path: &str) -> bool {
    // Trailing `*` (e.g. "/v1/foo/*") matches any suffix.
    if let Some(prefix) = pattern.strip_suffix("/*") {
        return path == prefix || path.starts_with(&format!("{}/", prefix));
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return path.starts_with(prefix);
    }

    let p_segs: Vec<&str> = pattern.split('/').collect();
    let path_segs: Vec<&str> = path.split('/').collect();

    let mut pi = 0;
    let mut si = 0;
    while pi < p_segs.len() {
        let p = p_segs[pi];
        // `{name:.*}` is a tail-wildcard: consumes the rest of the path. Must
        // be the last pattern segment per actix's routing model.
        if p.starts_with('{') && p.ends_with('}') && p.contains(":.*}") {
            // Need at least one remaining path segment to bind.
            return si < path_segs.len() && !path_segs[si].is_empty()
                && pi + 1 == p_segs.len();
        }
        if si >= path_segs.len() {
            return false;
        }
        let s = path_segs[si];
        if p.starts_with('{') && p.ends_with('}') {
            if s.is_empty() {
                return false;
            }
            // single-segment placeholder; fall through.
        } else if p != s {
            return false;
        }
        pi += 1;
        si += 1;
    }
    si == path_segs.len()
}

/// The composed route table consulted by cloud's auth middleware.
#[derive(Debug, Clone, Default)]
pub struct RouteAuthTable {
    rules: Vec<RouteRule>,
}

impl RouteAuthTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn builder() -> RouteAuthTableBuilder {
        RouteAuthTableBuilder::default()
    }

    pub fn from_rules(rules: Vec<RouteRule>) -> Self {
        Self { rules }
    }

    pub fn extend(&mut self, rules: impl IntoIterator<Item = RouteRule>) {
        self.rules.extend(rules);
    }

    pub fn rules(&self) -> &[RouteRule] {
        &self.rules
    }

    /// Find the most-specific rule that matches `(path, method)`. Returns
    /// `None` only if no rule matches at all — middleware MUST treat that as
    /// a fail-safe error, not a pass-through.
    pub fn lookup(&self, path: &str, method: HttpMethod) -> Option<&RouteRule> {
        self.rules
            .iter()
            .filter(|r| r.matches(path, method))
            .max_by_key(|r| r.specificity())
    }
}

/// Fluent builder for [`RouteAuthTable`].
#[derive(Default)]
pub struct RouteAuthTableBuilder {
    rules: Vec<RouteRule>,
}

impl RouteAuthTableBuilder {
    pub fn rule(mut self, rule: RouteRule) -> Self {
        self.rules.push(rule);
        self
    }

    pub fn public(self, path: impl Into<String>) -> Self {
        self.rule(RouteRule::public(path))
    }

    pub fn authenticated(
        self,
        path: impl Into<String>,
        methods: impl IntoIterator<Item = HttpMethod>,
    ) -> Self {
        self.rule(RouteRule::authenticated(path, methods))
    }

    pub fn perm(
        self,
        path: impl Into<String>,
        methods: impl IntoIterator<Item = HttpMethod>,
        kind: ResourceKind,
        action: Action,
    ) -> Self {
        self.rule(RouteRule::perm(path, methods, kind, action))
    }

    pub fn extend(mut self, rules: impl IntoIterator<Item = RouteRule>) -> Self {
        self.rules.extend(rules);
        self
    }

    pub fn build(self) -> RouteAuthTable {
        RouteAuthTable::from_rules(self.rules)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_set_bitops() {
        let mut s = ActionSet::empty();
        assert!(s.is_empty());
        s.insert(Action::Read);
        assert!(s.contains(Action::Read));
        assert!(!s.contains(Action::Write));
        s.insert(Action::Write);
        assert!(s.contains(Action::Write));
        s.remove(Action::Read);
        assert!(!s.contains(Action::Read));
        assert!(s.contains(Action::Write));

        let all = ActionSet::all();
        assert!(all.contains(Action::Read));
        assert!(all.contains(Action::Write));
        assert!(all.contains(Action::Manage));
        assert!(all.contains(Action::Execute));

        let rw = ActionSet::from([Action::Read, Action::Write]);
        assert!(rw.is_subset_of(all));
        assert!(!all.is_subset_of(rw));
    }

    #[test]
    fn action_set_serde_is_u8() {
        let s = ActionSet::from([Action::Read, Action::Write]);
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "3");
        let back: ActionSet = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn permissions_grant_revoke_allows() {
        let mut p = Permissions::empty();
        assert!(!p.allows(ResourceKind::Agent, Action::Read));

        p.grant(ResourceKind::Agent, ActionSet::from(Action::Read));
        assert!(p.allows(ResourceKind::Agent, Action::Read));
        assert!(!p.allows(ResourceKind::Agent, Action::Write));

        p.grant(
            ResourceKind::Agent,
            ActionSet::from([Action::Write, Action::Manage]),
        );
        assert!(p.allows(ResourceKind::Agent, Action::Write));
        assert!(p.allows(ResourceKind::Agent, Action::Manage));

        p.revoke(ResourceKind::Agent, ActionSet::from(Action::Manage));
        assert!(!p.allows(ResourceKind::Agent, Action::Manage));
        assert!(p.allows(ResourceKind::Agent, Action::Read));
    }

    #[test]
    fn permissions_intersect_attenuates() {
        let mut caller = Permissions::empty();
        caller.grant(
            ResourceKind::Agent,
            ActionSet::from([Action::Read, Action::Write, Action::Manage]),
        );
        caller.grant(ResourceKind::Note, ActionSet::from(Action::Read));

        let mut requested = Permissions::empty();
        requested.grant(
            ResourceKind::Agent,
            ActionSet::from([Action::Read, Action::Execute]),
        );
        requested.grant(ResourceKind::Workflow, ActionSet::from(Action::Read));

        let granted = requested.intersect(&caller);
        // Agent: requested Read+Execute, caller has Read+Write+Manage → Read
        assert!(granted.allows(ResourceKind::Agent, Action::Read));
        assert!(!granted.allows(ResourceKind::Agent, Action::Execute));
        assert!(!granted.allows(ResourceKind::Agent, Action::Write));
        // Workflow: caller doesn't have it → empty
        assert!(!granted.allows(ResourceKind::Workflow, Action::Read));
        // Note: not requested → not granted
        assert!(!granted.allows(ResourceKind::Note, Action::Read));
    }

    #[test]
    fn permissions_subset() {
        let mut a = Permissions::empty();
        a.grant(ResourceKind::Agent, ActionSet::from(Action::Read));
        let mut b = Permissions::empty();
        b.grant(
            ResourceKind::Agent,
            ActionSet::from([Action::Read, Action::Write]),
        );

        assert!(a.is_subset_of(&b));
        assert!(!b.is_subset_of(&a));
        assert!(Permissions::empty().is_subset_of(&a));
    }

    #[test]
    fn permissions_serde_round_trip() {
        let mut p = Permissions::empty();
        p.grant(
            ResourceKind::Agent,
            ActionSet::from([Action::Read, Action::Write]),
        );
        p.grant(
            ResourceKind::Completion,
            ActionSet::from(Action::Execute),
        );
        let json = serde_json::to_string(&p).unwrap();
        let back: Permissions = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn permissions_defaults() {
        let user = Permissions::default_for_user_scope();
        assert!(user.allows(ResourceKind::Completion, Action::Execute));
        assert!(user.allows(ResourceKind::ConnectionUser, Action::Read));
        assert!(user.allows(ResourceKind::ConnectionUser, Action::Write));
        assert!(!user.allows(ResourceKind::ConnectionWorkspace, Action::Read));
        assert!(!user.allows(ResourceKind::Agent, Action::Read));

        let ws = Permissions::default_for_workspace_member();
        assert!(ws.allows(ResourceKind::Agent, Action::Manage));
        assert!(ws.allows(ResourceKind::Mcp, Action::Execute));
        assert!(ws.allows(ResourceKind::ConnectionWorkspace, Action::Manage));
        assert!(!ws.allows(ResourceKind::ConnectionUser, Action::Write));

        let completion = Permissions::default_for_completion();
        assert!(completion.allows(ResourceKind::Completion, Action::Execute));
        assert!(!completion.allows(ResourceKind::Agent, Action::Read));
    }

    #[test]
    fn auth_context_require_checks() {
        let ctx = AuthContext::local_dev();
        assert!(ctx.require(ResourceKind::Agent, Action::Manage).is_ok());

        let mut empty = AuthContext::empty_for_test();
        empty.scope = AuthScope::Workspace;
        empty.identity.workspace_id = Some(Uuid::nil());
        assert_eq!(
            empty.require(ResourceKind::Agent, Action::Read),
            Err(AuthError::MissingPermission {
                kind: ResourceKind::Agent,
                action: Action::Read
            })
        );
        assert!(empty.require_workspace(Uuid::nil()).is_ok());
        let other = Uuid::from_u128(1);
        assert_eq!(
            empty.require_workspace(other),
            Err(AuthError::WrongWorkspace {
                expected: other,
                actual: Some(Uuid::nil())
            })
        );
    }

    #[test]
    fn auth_op_parts() {
        let op = AuthOp::Read {
            kind: ResourceKind::Agent,
            id: Some(Uuid::nil()),
        };
        assert_eq!(op.action(), Action::Read);
        assert_eq!(op.kind(), ResourceKind::Agent);
        assert_eq!(op.id(), Some(Uuid::nil()));
    }

    #[test]
    fn route_pattern_matching() {
        assert!(pattern_matches("/v1/foo", "/v1/foo"));
        assert!(!pattern_matches("/v1/foo", "/v1/foo/bar"));
        assert!(pattern_matches("/v1/foo/*", "/v1/foo/bar"));
        assert!(pattern_matches("/v1/foo/*", "/v1/foo/bar/baz"));
        assert!(pattern_matches("/v1/foo/*", "/v1/foo"));
        assert!(!pattern_matches("/v1/foo/*", "/v1/other"));
        assert!(pattern_matches("/v1/agents/{id}", "/v1/agents/abc"));
        assert!(!pattern_matches("/v1/agents/{id}", "/v1/agents"));
        assert!(pattern_matches(
            "/v1/agents/{id}/tasks/{tid}",
            "/v1/agents/x/tasks/y"
        ));
        assert!(pattern_matches(
            "/v1/agents/{id:.*}",
            "/v1/agents/some/deep/path"
        ));
    }

    #[test]
    fn route_table_lookup_picks_most_specific() {
        let table = RouteAuthTable::builder()
            .perm(
                "/v1/agents/*",
                [HttpMethod::Any],
                ResourceKind::Agent,
                Action::Read,
            )
            .perm(
                "/v1/agents/{id}",
                [HttpMethod::Post],
                ResourceKind::Completion,
                Action::Execute,
            )
            .build();

        let r = table.lookup("/v1/agents/abc", HttpMethod::Post).unwrap();
        assert_eq!(
            r.requirement,
            RouteRequirement::Permission {
                kind: ResourceKind::Completion,
                action: Action::Execute,
            }
        );

        let r2 = table.lookup("/v1/agents/abc", HttpMethod::Get).unwrap();
        assert_eq!(
            r2.requirement,
            RouteRequirement::Permission {
                kind: ResourceKind::Agent,
                action: Action::Read,
            }
        );

        assert!(table.lookup("/v1/unknown", HttpMethod::Get).is_none());
    }

    #[test]
    fn route_requirement_check_attenuated() {
        let mut perms = Permissions::empty();
        perms.grant(ResourceKind::Agent, ActionSet::from(Action::Read));
        let req = RouteRequirement::Permission {
            kind: ResourceKind::Agent,
            action: Action::Read,
        };
        assert!(req.check(&perms).is_ok());
        let req2 = RouteRequirement::Permission {
            kind: ResourceKind::Agent,
            action: Action::Manage,
        };
        assert!(req2.check(&perms).is_err());
    }

    #[test]
    fn auth_error_http_status() {
        assert_eq!(AuthError::Unauthenticated.http_status(), 401);
        assert_eq!(
            AuthError::MissingPermission {
                kind: ResourceKind::Agent,
                action: Action::Read,
            }
            .http_status(),
            403
        );
        assert_eq!(AuthError::NotConfigured.http_status(), 500);
        assert_eq!(
            AuthError::NoRuleForRoute {
                path: "/x".into(),
                method: "GET".into(),
            }
            .http_status(),
            500
        );
    }

    #[derive(Default)]
    struct PermissiveStore;
    #[async_trait::async_trait]
    impl Authorize for PermissiveStore {}

    #[derive(Default)]
    struct OwnerOnlyStore {
        owner: Uuid,
    }
    #[async_trait::async_trait]
    impl Authorize for OwnerOnlyStore {
        async fn authorize(
            &self,
            ctx: &AuthContext,
            op: &AuthOp,
        ) -> Result<(), AuthError> {
            ctx.require(op.kind(), op.action())?;
            ctx.require_owner(self.owner)
        }
    }

    #[tokio::test]
    async fn authorize_default_is_permissive() {
        let store = PermissiveStore;
        let ctx = AuthContext::empty_for_test();
        let op = AuthOp::Read {
            kind: ResourceKind::Agent,
            id: None,
        };
        assert!(store.authorize(&ctx, &op).await.is_ok());
    }

    #[tokio::test]
    async fn authorize_override_enforces() {
        let owner = Uuid::from_u128(42);
        let store = OwnerOnlyStore { owner };
        let mut perms = Permissions::empty();
        perms.grant(ResourceKind::Agent, ActionSet::from(Action::Read));
        let mut ctx = AuthContext::empty_for_test();
        ctx.permissions = perms.clone();
        ctx.identity.user_id = owner;
        let op = AuthOp::Read {
            kind: ResourceKind::Agent,
            id: None,
        };
        assert!(store.authorize(&ctx, &op).await.is_ok());

        // Wrong user → NotOwner
        ctx.identity.user_id = Uuid::from_u128(1);
        assert_eq!(
            store.authorize(&ctx, &op).await,
            Err(AuthError::NotOwner)
        );

        // Missing permission → MissingPermission
        ctx.identity.user_id = owner;
        ctx.permissions = Permissions::empty();
        assert_eq!(
            store.authorize(&ctx, &op).await,
            Err(AuthError::MissingPermission {
                kind: ResourceKind::Agent,
                action: Action::Read
            })
        );
    }
}
