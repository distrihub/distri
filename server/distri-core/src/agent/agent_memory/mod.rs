//! Agent memory at run time (distrihub/distri-cloud#304, phase 1).
//!
//! An agent with `[memory]` gets a [`MemorySession`] when its run is built
//! (`create_agent_from_config`): the space it uses, its config, the scope resolved from the
//! run's input, and the URIs read in each task (read-before-edit). At the start of the run
//! [`prepare_run`] refreshes the folder rollups, renders the memory block (rules first, then
//! folders by tier within `inject_tokens`), puts it in the system prompt through the `memory`
//! partial, and emits `memory_injected`.
//!
//! - [`tools`]: `memory_read`, `memory_search`, `memory_write`, `memory_forget`.
//! - [`reflect`]: turns waiting signals into a validated plan of memory changes.
//! - [`step`]: the workflow `memory` step (signal / write / forget, no LLM).

pub mod reflect;
pub mod step;
pub mod tools;

use std::collections::HashSet;
use std::sync::{Arc, Mutex, RwLock};

use distri_types::memory::{
    inject::{render_memory_block, InjectionInput},
    MemoryAuthor, MemoryConfig, MemoryFolder, MemoryItem, MemoryStatus, MemoryUri, L0_MAX_CHARS,
    L1_MAX_CHARS, NO_SCOPE,
};
use distri_types::{AgentEventType, Message};

use crate::agent::ExecutorContext;
use crate::AgentError;

/// The `dynamic_values` key the memory block travels under to the formatter.
pub const MEMORY_BLOCK_KEY: &str = "memory_block";
/// The workspace key of a host without workspaces (the OSS server, the CLI).
pub const LOCAL_WORKSPACE: &str = "local";

/// The workspace memory is kept under for this run.
pub fn workspace_key(ctx: &ExecutorContext) -> String {
    ctx.workspace_id
        .clone()
        .filter(|w| !w.is_empty())
        .or_else(|| distri_types::context::current_workspace_id().map(|u| u.to_string()))
        .unwrap_or_else(|| LOCAL_WORKSPACE.to_string())
}

/// Who a run's memory acts for (security review of #304).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryCaller {
    /// A workspace member, a workspace API key, a workflow, the CLI, or the host itself: the
    /// agent's `[memory].scope` template applies.
    Trusted,
    /// A proven end user (an end-user token, an embed visitor or a channel user with a known
    /// id): memory is forced into their own `user:<id>` scope.
    EndUser(String),
    /// An end-user surface without a proven id: no memory at all.
    Unidentified,
}

impl MemoryCaller {
    /// The scope an end user is held to.
    pub fn forced_scope(&self) -> Option<String> {
        match self {
            MemoryCaller::EndUser(id) => Some(sanitize_segment(&format!("user:{id}"))),
            _ => None,
        }
    }
}

/// Classify the run's caller from its [`distri_types::RequestCaller`]. The host sets the
/// `end_user` attribute only when the credential proves it (`user:<id>` for end-user tokens,
/// `u:<id>` for channel users); an API key's self-declared end user is not one.
pub fn memory_caller(ctx: &ExecutorContext) -> MemoryCaller {
    use distri_types::CallerKind;
    let Some(caller) = ctx.caller.as_ref() else {
        return MemoryCaller::Trusted;
    };
    let end_user = caller
        .attributes
        .get("end_user")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    match caller.kind {
        CallerKind::Dashboard | CallerKind::Cli | CallerKind::Workflow => {
            match end_user.and_then(|e| e.strip_prefix("user:")) {
                Some(id) if !id.is_empty() => MemoryCaller::EndUser(id.to_string()),
                _ => MemoryCaller::Trusted,
            }
        }
        CallerKind::PublicClient | CallerKind::Bot => match end_user {
            Some(e) => {
                let id = e.strip_prefix("user:").unwrap_or(e);
                if id.is_empty() {
                    MemoryCaller::Unidentified
                } else {
                    MemoryCaller::EndUser(id.to_string())
                }
            }
            None => MemoryCaller::Unidentified,
        },
    }
}

/// One agent's memory for one run.
pub struct MemorySession {
    pub workspace_id: String,
    pub agent: String,
    pub space: String,
    pub config: MemoryConfig,
    /// Set for end users: the only scope this run reads and writes.
    pub forced_scope: Option<String>,
    /// A trusted caller (see [`MemoryCaller`]).
    pub trusted: bool,
    scope: RwLock<String>,
    /// `task_id|uri` of every item read in a task.
    read: Mutex<HashSet<String>>,
}

impl std::fmt::Debug for MemorySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemorySession")
            .field("agent", &self.agent)
            .field("space", &self.space)
            .field("scope", &self.scope())
            .finish()
    }
}

impl MemorySession {
    pub fn new(workspace_id: String, agent: &str, config: MemoryConfig) -> Self {
        Self {
            workspace_id,
            agent: agent.to_string(),
            space: config.space_for(agent),
            config,
            forced_scope: None,
            trusted: true,
            scope: RwLock::new(NO_SCOPE.to_string()),
            read: Mutex::new(HashSet::new()),
        }
    }

    /// The session for `caller`: an end user is held to their own scope.
    pub fn for_caller(mut self, caller: &MemoryCaller) -> Self {
        self.trusted = matches!(caller, MemoryCaller::Trusted);
        self.forced_scope = caller.forced_scope();
        if let Some(scope) = &self.forced_scope {
            self.set_scope(scope.clone());
        }
        self
    }

    /// Whether this run may write memory: never read-only agents, and end users only when the
    /// agent sets `end_users_write`.
    pub fn can_write(&self) -> bool {
        !self.config.read_only && (self.trusted || self.config.end_users_write)
    }

    pub fn scope(&self) -> String {
        self.scope.read().unwrap().clone()
    }

    pub fn set_scope(&self, scope: String) {
        *self.scope.write().unwrap() = scope;
    }

    pub fn mark_read(&self, task_id: &str, uri: &MemoryUri) {
        self.read.lock().unwrap().insert(format!("{task_id}|{uri}"));
    }

    pub fn was_read(&self, task_id: &str, uri: &MemoryUri) -> bool {
        self.read
            .lock()
            .unwrap()
            .contains(&format!("{task_id}|{uri}"))
    }

    pub fn author(&self, task_id: &str) -> MemoryAuthor {
        MemoryAuthor::Agent {
            agent: Some(self.agent.clone()),
            task_id: Some(task_id.to_string()).filter(|t| !t.is_empty()),
        }
    }

    /// A URI the agent gave: `mem://…` or `folder/slug` in its own space and scope.
    pub fn uri(&self, s: &str) -> Result<MemoryUri, String> {
        MemoryUri::parse_relative(s, &self.space, &self.scope()).map_err(|e| e.to_string())
    }

    pub fn in_scope(&self, uri: &MemoryUri) -> bool {
        uri.space == self.space && uri.scope == self.scope()
    }
}

/// The memory session of the agent this context runs, if it has `[memory]`.
pub async fn session_of(ctx: &ExecutorContext) -> Option<Arc<MemorySession>> {
    ctx.hook_prompt_state.read().await.memory.clone()
}

/// Give the context's agent a memory session, or take it away (and its block) with `None`, so
/// an agent without `[memory]` never inherits another's.
pub async fn set_session(ctx: &ExecutorContext, session: Option<Arc<MemorySession>>) {
    let mut st = ctx.hook_prompt_state.write().await;
    if session.is_none() {
        st.dynamic_values.remove(MEMORY_BLOCK_KEY);
    }
    st.memory = session;
}

/// A URI segment from free text: characters a `mem://` segment does not allow become `-`.
pub fn sanitize_segment(s: &str) -> String {
    let out: String = s
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':') {
                c
            } else {
                '-'
            }
        })
        .collect();
    out.trim_matches('-').chars().take(200).collect()
}

/// Resolve a `[memory].scope` template (`"{input.account}"`) against a run's input. A missing
/// template, an empty result or an unresolved placeholder is the "no scope" scope (`_`).
///
/// `{user.id}` is the run's user. Only trusted callers reach this: an end user's scope is
/// forced (see [`MemoryCaller`]).
pub fn resolve_scope(
    template: Option<&str>,
    input: &serde_json::Value,
    user_id: Option<&str>,
) -> String {
    let Some(t) = template.map(str::trim).filter(|t| !t.is_empty()) else {
        return NO_SCOPE.to_string();
    };
    let t = match user_id.filter(|u| !u.is_empty()) {
        Some(u) => t.replace("{user.id}", u),
        None => t.to_string(),
    };
    let ctx = serde_json::json!({ "input": input });
    let resolved = distri_workflow::resolve::resolve_template(&t, &ctx);
    if resolved.contains('{') || resolved.contains('}') {
        return NO_SCOPE.to_string();
    }
    let s = sanitize_segment(&resolved);
    if s.is_empty() {
        NO_SCOPE.to_string()
    } else {
        s
    }
}

/// A run's input: the message text as JSON when it is JSON, else the text.
pub fn message_input(message: &Message) -> serde_json::Value {
    let text = message.as_text().unwrap_or_default();
    serde_json::from_str(text.trim()).unwrap_or(serde_json::Value::String(text))
}

/// A folder's rollup from its items, without an LLM: L0 names the first items, L1 lists each
/// item's overview, whole lines only.
pub fn folder_rollup(folder: &str, items: &[&MemoryItem]) -> (String, String) {
    let mut l0 = String::new();
    for item in items.iter().take(3) {
        let part = item.l0.trim();
        let next = if l0.is_empty() {
            part.to_string()
        } else {
            format!("{l0}; {part}")
        };
        if next.chars().count() > L0_MAX_CHARS {
            break;
        }
        l0 = next;
    }
    if l0.is_empty() {
        l0 = folder.to_string();
    }
    let mut l1 = String::new();
    for item in items {
        let overview = item
            .l1
            .as_deref()
            .and_then(|t| t.lines().map(str::trim).find(|l| !l.is_empty()))
            .unwrap_or(item.l0.trim());
        let slug = item.uri.slug.as_deref().unwrap_or_default();
        let line = format!("{slug}: {overview}\n");
        if l1.chars().count() + line.chars().count() > L1_MAX_CHARS {
            break;
        }
        l1.push_str(&line);
    }
    (l0, l1.trim_end().to_string())
}

/// Refresh the rollups of folders whose items changed since (10% or more of the folder, or no
/// rollup yet). Returns the folders as they now are.
pub async fn refresh_rollups(
    store: &dyn distri_types::AgentMemoryStore,
    ws: &str,
    space: &str,
    scope: &str,
    mut folders: Vec<MemoryFolder>,
    items: &[MemoryItem],
) -> anyhow::Result<Vec<MemoryFolder>> {
    let mut names: Vec<String> = items.iter().map(|i| i.uri.folder.clone()).collect();
    names.sort();
    names.dedup();
    for name in names {
        let active: Vec<&MemoryItem> = items
            .iter()
            .filter(|i| i.uri.folder == name && i.status == MemoryStatus::Active)
            .collect();
        let row = folders.iter().find(|f| f.uri.folder == name);
        let stale = match row {
            None => true,
            Some(f) => {
                f.l1.is_empty()
                    || (f.pending_changes > 0
                        && (f.pending_changes as usize) * 10 >= (f.item_count.max(1) as usize))
            }
        };
        if !stale {
            continue;
        }
        let (l0, l1) = folder_rollup(&name, &active);
        let uri = MemoryUri::folder(space, scope, &name);
        store.set_folder_summary(ws, &uri, &l0, &l1).await?;
        match folders.iter_mut().find(|f| f.uri.folder == name) {
            Some(f) => {
                f.l0 = l0;
                f.l1 = l1;
                f.pending_changes = 0;
                f.item_count = active.len() as i32;
            }
            None => folders.push(MemoryFolder {
                uri,
                agent: None,
                l0,
                l1,
                item_count: active.len() as i32,
                pending_changes: 0,
                updated_at: chrono::Utc::now().timestamp_millis(),
            }),
        }
    }
    Ok(folders)
}

/// At the start of a run: resolve the scope, reflect when due (batched), render the memory
/// block into the system prompt and record what was injected.
pub async fn prepare_run(
    session: &MemorySession,
    message: &Message,
    ctx: &Arc<ExecutorContext>,
) -> Result<(), AgentError> {
    let Some(orch) = ctx.orchestrator.clone() else {
        return Ok(());
    };
    let scope = match &session.forced_scope {
        Some(forced) => forced.clone(),
        None => resolve_scope(
            session.config.scope.as_deref(),
            &message_input(message),
            Some(ctx.user_id.as_str()),
        ),
    };
    session.set_scope(scope.clone());
    let ws = session.workspace_id.clone();

    if session.config.reflect == distri_types::ReflectMode::Batched {
        let req = reflect::ReflectRequest {
            workspace_id: ws.clone(),
            space: session.space.clone(),
            scope: scope.clone(),
            user_id: Some(ctx.user_id.clone()).filter(|u| !u.is_empty()),
            default_model_settings: ctx.default_model_settings.clone(),
        };
        if let Err(e) = reflect::reflect_if_due(&orch, req, &session.config).await {
            tracing::warn!(space = %session.space, %scope, error = %e, "memory: batched reflection failed");
        }
    }

    let store = orch.agent_memory_store.clone();
    let mem_err = |e: anyhow::Error| AgentError::Session(format!("memory: {e}"));
    let (folders, items) = store
        .index(&ws, &session.space, &scope)
        .await
        .map_err(mem_err)?;
    let folders = refresh_rollups(store.as_ref(), &ws, &session.space, &scope, folders, &items)
        .await
        .map_err(mem_err)?;
    let active: Vec<MemoryItem> = items
        .into_iter()
        .filter(|i| i.status == MemoryStatus::Active)
        .collect();
    let out = render_memory_block(&InjectionInput {
        space: &session.space,
        scope: &scope,
        rules: &session.config.rules,
        folders: &folders,
        items: &active,
        budget_tokens: session.config.inject_tokens(),
    });

    {
        let mut st = ctx.hook_prompt_state.write().await;
        st.dynamic_values.insert(
            MEMORY_BLOCK_KEY.to_string(),
            serde_json::Value::String(out.text.clone()),
        );
    }
    let used: Vec<MemoryUri> = out
        .entries
        .iter()
        .filter_map(|e| e.uri.parse::<MemoryUri>().ok())
        .filter(|u| u.is_item())
        .collect();
    if !used.is_empty() {
        if let Err(e) = store.mark_used(&ws, &used).await {
            tracing::warn!(error = %e, "memory: mark_used failed");
        }
    }
    ctx.emit(AgentEventType::MemoryInjected {
        space: session.space.clone(),
        scope,
        uris: out.entries.iter().map(|e| e.uri.clone()).collect(),
        tiers: out.entries.iter().map(|e| e.tier.clone()).collect(),
        tokens: out.tokens,
    })
    .await;
    Ok(())
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn scope_templates_resolve_or_fall_back() {
        let input = serde_json::json!({"account": "reddit:RadishGlobal341", "n": 2});
        assert_eq!(
            resolve_scope(Some("{input.account}"), &input, None),
            "reddit:RadishGlobal341"
        );
        assert_eq!(resolve_scope(None, &input, None), "_");
        assert_eq!(resolve_scope(Some("{input.missing}"), &input, None), "_");
        assert_eq!(
            resolve_scope(Some("hn {input.n}"), &input, None),
            "hn-2",
            "spaces become dashes"
        );
        assert_eq!(
            resolve_scope(
                Some("{input.account}"),
                &serde_json::json!("free text"),
                None
            ),
            "_"
        );
        assert_eq!(
            resolve_scope(Some("u-{user.id}"), &input, Some("42")),
            "u-42"
        );
    }

    #[test]
    fn callers_are_classified() {
        use distri_types::{CallerKind, RequestCaller};
        let caller = |kind, end_user: Option<&str>| {
            let mut ctx = ExecutorContext::default();
            ctx.caller = Some(RequestCaller {
                kind,
                source_id: None,
                origin: None,
                client: None,
                attributes: end_user
                    .map(|e| [("end_user".to_string(), e.to_string())].into())
                    .unwrap_or_default(),
            });
            memory_caller(&ctx)
        };
        assert_eq!(
            memory_caller(&ExecutorContext::default()),
            MemoryCaller::Trusted
        );
        assert_eq!(caller(CallerKind::Dashboard, None), MemoryCaller::Trusted);
        assert_eq!(
            caller(CallerKind::Dashboard, Some("api:k1:bob")),
            MemoryCaller::Trusted,
            "an API key's named end user is the workspace acting"
        );
        assert_eq!(
            caller(CallerKind::Dashboard, Some("user:alice")),
            MemoryCaller::EndUser("alice".into())
        );
        assert_eq!(
            caller(CallerKind::PublicClient, None),
            MemoryCaller::Unidentified
        );
        assert_eq!(
            caller(CallerKind::Bot, Some("u:7")),
            MemoryCaller::EndUser("u:7".into())
        );
        assert_eq!(
            MemoryCaller::EndUser("u:7".into())
                .forced_scope()
                .as_deref(),
            Some("user:u:7")
        );
    }
}
