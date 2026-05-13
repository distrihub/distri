//! In-process store implementations for distri-server.
//!
//! These are used when no persistent store backend is configured (e.g. in
//! tests or single-tenant local runs that haven't yet been wired to SQLite).
//! They are **not** recommended for production; Task 5 adds the SQLite-backed
//! implementations to distri-stores.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use distri_types::api::notes::{CreateNoteRequest, ListNotesQuery, NoteRecord, UpdateNoteRequest};
use distri_types::api::spans::{SpanRecord, TraceRecord};
use distri_types::connections::{Connection, ConnectionStatus, NewConnection};
use distri_types::credentials::CredentialToken;
use distri_types::stores::{
    ConnectionStore, CredentialTokenStore, NoteStore, SpanQuery, SpanStore,
};
use tokio::sync::RwLock;
use uuid::Uuid;

// ── In-memory ConnectionStore ──────────────────────────────────────────────

pub struct InMemoryConnectionStore {
    connections: RwLock<HashMap<Uuid, Connection>>,
}

impl InMemoryConnectionStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            connections: RwLock::new(HashMap::new()),
        })
    }
}

impl Default for InMemoryConnectionStore {
    fn default() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl ConnectionStore for InMemoryConnectionStore {
    async fn create(&self, new_conn: NewConnection) -> anyhow::Result<Connection> {
        let now = chrono::Utc::now();
        let conn = Connection {
            id: Uuid::new_v4(),
            workspace_id: new_conn.workspace_id,
            skill_id: new_conn.skill_id,
            name: new_conn.name,
            status: new_conn.status,
            config: new_conn.config,
            connected_by: new_conn.connected_by,
            created_at: now,
            updated_at: now,
            auth_scope: new_conn.auth_scope,
            credential_id: new_conn.credential_id,
            kind: new_conn.kind,
            is_system: new_conn.is_system,
        };
        self.connections.write().await.insert(conn.id, conn.clone());
        Ok(conn)
    }

    async fn get_by_id(&self, id: &str) -> anyhow::Result<Option<Connection>> {
        let id = Uuid::parse_str(id).map_err(|e| anyhow::anyhow!("invalid UUID: {}", e))?;
        Ok(self.connections.read().await.get(&id).cloned())
    }

    async fn list_by_workspace(&self, _workspace_id: &str) -> anyhow::Result<Vec<Connection>> {
        // In single-tenant mode all connections belong to the one workspace.
        Ok(self.connections.read().await.values().cloned().collect())
    }

    async fn update_status(&self, id: &str, status: ConnectionStatus) -> anyhow::Result<()> {
        let id = Uuid::parse_str(id).map_err(|e| anyhow::anyhow!("invalid UUID: {}", e))?;
        let mut map = self.connections.write().await;
        if let Some(conn) = map.get_mut(&id) {
            conn.status = status;
            conn.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn update_skill_id(&self, id: &str, skill_id: Uuid) -> anyhow::Result<()> {
        let id = Uuid::parse_str(id).map_err(|e| anyhow::anyhow!("invalid UUID: {}", e))?;
        let mut map = self.connections.write().await;
        if let Some(conn) = map.get_mut(&id) {
            conn.skill_id = skill_id;
            conn.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn update(
        &self,
        id: &str,
        name: Option<String>,
    ) -> anyhow::Result<Connection> {
        let id = Uuid::parse_str(id).map_err(|e| anyhow::anyhow!("invalid UUID: {}", e))?;
        let mut map = self.connections.write().await;
        let conn = map
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("connection not found"))?;
        if let Some(n) = name {
            conn.name = n;
        }
        conn.updated_at = chrono::Utc::now();
        Ok(conn.clone())
    }

    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        let id = Uuid::parse_str(id).map_err(|e| anyhow::anyhow!("invalid UUID: {}", e))?;
        self.connections.write().await.remove(&id);
        Ok(())
    }

    async fn get_by_provider(
        &self,
        _workspace_id: &str,
        provider: &str,
    ) -> anyhow::Result<Option<Connection>> {
        // OSS in-memory store doesn't carry the credential link; match by
        // connection name as a fallback. Cloud's `PgConnectionStore` joins
        // through the `credentials.material->>'provider'`.
        let map = self.connections.read().await;
        Ok(map.values().find(|c| c.name == provider).cloned())
    }
}

// ── In-memory CredentialTokenStore ───────────────────────────────────────────

pub struct InMemoryCredentialTokenStore {
    tokens: RwLock<HashMap<String, CredentialToken>>,
    oauth_states: RwLock<HashMap<String, serde_json::Value>>,
}

impl InMemoryCredentialTokenStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            tokens: RwLock::new(HashMap::new()),
            oauth_states: RwLock::new(HashMap::new()),
        })
    }
}

impl Default for InMemoryCredentialTokenStore {
    fn default() -> Self {
        Self {
            tokens: RwLock::new(HashMap::new()),
            oauth_states: RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl CredentialTokenStore for InMemoryCredentialTokenStore {
    async fn store_token(&self, connection_id: &str, token: CredentialToken) -> anyhow::Result<()> {
        self.tokens
            .write()
            .await
            .insert(connection_id.to_string(), token);
        Ok(())
    }

    async fn get_token(&self, connection_id: &str) -> anyhow::Result<Option<CredentialToken>> {
        Ok(self.tokens.read().await.get(connection_id).cloned())
    }

    async fn remove_token(&self, connection_id: &str) -> anyhow::Result<()> {
        self.tokens.write().await.remove(connection_id);
        Ok(())
    }

    async fn store_oauth_state(
        &self,
        state_key: &str,
        state: serde_json::Value,
    ) -> anyhow::Result<()> {
        self.oauth_states
            .write()
            .await
            .insert(state_key.to_string(), state);
        Ok(())
    }

    async fn get_oauth_state(&self, state_key: &str) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(self.oauth_states.read().await.get(state_key).cloned())
    }

    async fn remove_oauth_state(&self, state_key: &str) -> anyhow::Result<()> {
        self.oauth_states.write().await.remove(state_key);
        Ok(())
    }
}

// ── In-memory SpanStore ───────────────────────────────────────────────────────

/// In-process span store for OSS distri-server.
///
/// Retains all spans for the lifetime of the process (no persistence across
/// restarts).  The distri CLI uses this to record OTel spans emitted during
/// agent execution so that `distri traces` can display them locally.
pub struct InMemorySpanStore {
    spans: RwLock<Vec<SpanRecord>>,
}

impl InMemorySpanStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            spans: RwLock::new(Vec::new()),
        })
    }
}

impl Default for InMemorySpanStore {
    fn default() -> Self {
        Self {
            spans: RwLock::new(Vec::new()),
        }
    }
}

#[async_trait]
impl SpanStore for InMemorySpanStore {
    async fn bulk_insert(&self, new_spans: Vec<SpanRecord>) -> anyhow::Result<usize> {
        let mut store = self.spans.write().await;
        let mut inserted = 0usize;
        for span in new_spans {
            // Idempotent: skip if (trace_id, span_id) already present
            let exists = store
                .iter()
                .any(|s| s.trace_id == span.trace_id && s.span_id == span.span_id);
            if !exists {
                store.push(span);
                inserted += 1;
            }
        }
        Ok(inserted)
    }

    async fn list_spans(
        &self,
        _workspace_id: &str,
        query: SpanQuery,
    ) -> anyhow::Result<Vec<SpanRecord>> {
        let store = self.spans.read().await;
        let mut result: Vec<SpanRecord> = match query {
            SpanQuery::ByThreadId(_) => {
                // In-memory store doesn't track thread_id on span records (they
                // come from the OTel exporter which doesn't carry thread context).
                // Return empty to avoid surprising results.
                vec![]
            }
            SpanQuery::ByTraceId(trace_id) => store
                .iter()
                .filter(|s| s.trace_id == trace_id)
                .cloned()
                .collect(),
        };
        result.sort_by_key(|s| s.start_time_ns);
        Ok(result)
    }

    async fn list_traces(
        &self,
        _workspace_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<TraceRecord>> {
        let store = self.spans.read().await;

        // Group spans by trace_id and find root spans (no parent_span_id)
        let mut trace_map: HashMap<String, Vec<&SpanRecord>> = HashMap::new();
        for span in store.iter() {
            trace_map
                .entry(span.trace_id.clone())
                .or_default()
                .push(span);
        }

        let mut records: Vec<TraceRecord> = trace_map
            .into_iter()
            .filter_map(|(trace_id, spans)| {
                // Root span = no parent_span_id or empty parent_span_id
                let root = spans.iter().find(|s| {
                    s.parent_span_id
                        .as_deref()
                        .map(|p| p.is_empty())
                        .unwrap_or(true)
                })?;

                let span_count = spans.len() as i64;
                let start_time_ns = spans.iter().map(|s| s.start_time_ns).min()?;
                let end_time_ns = spans.iter().map(|s| s.end_time_ns).max()?;

                Some(TraceRecord {
                    trace_id,
                    name: root.name.clone(),
                    start_time_ns,
                    end_time_ns,
                    span_count,
                    thread_id: None,
                    input_tokens: 0,
                    total_cost: 0.0,
                    step_count: 0,
                    models: vec![],
                    input_preview: None,
                })
            })
            .collect();

        // Sort by start_time_ns descending (most recent first)
        records.sort_by(|a, b| b.start_time_ns.cmp(&a.start_time_ns));
        records.truncate(limit as usize);

        Ok(records)
    }
}

// ── In-memory NoteStore ───────────────────────────────────────────────────────

/// In-process note store for OSS distri-server tests and dev mode.
///
/// Notes are stored in a `HashMap` keyed by UUID and are discarded on process
/// exit.  Production deployments should use the SQLite-backed `DieselNoteStore`.
pub struct InMemoryNoteStore {
    notes: RwLock<HashMap<Uuid, NoteRecord>>,
}

impl InMemoryNoteStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            notes: RwLock::new(HashMap::new()),
        })
    }
}

impl Default for InMemoryNoteStore {
    fn default() -> Self {
        Self {
            notes: RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl NoteStore for InMemoryNoteStore {
    async fn list(&self, query: &ListNotesQuery) -> anyhow::Result<Vec<NoteRecord>> {
        let map = self.notes.read().await;
        let mut records: Vec<NoteRecord> = map.values().cloned().collect();

        // Tag filter
        if let Some(tag) = &query.tag {
            records.retain(|n| n.tags.iter().any(|t| t == tag));
        }

        // Search filter
        if let Some(search) = &query.search {
            let lower = search.to_lowercase();
            records.retain(|n| {
                n.title.to_lowercase().contains(&lower) || n.content.to_lowercase().contains(&lower)
            });
        }

        // Sort by updated_at descending
        records.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(records)
    }

    async fn get(&self, id: Uuid) -> anyhow::Result<Option<NoteRecord>> {
        Ok(self.notes.read().await.get(&id).cloned())
    }

    async fn create(&self, req: CreateNoteRequest) -> anyhow::Result<NoteRecord> {
        let now = chrono::Utc::now();
        let note = NoteRecord {
            id: Uuid::new_v4(),
            workspace_id: Uuid::nil(),
            title: req.title,
            content: req.content,
            tags: req.tags,
            created_by: None,
            created_at: now,
            updated_at: now,
        };
        self.notes.write().await.insert(note.id, note.clone());
        Ok(note)
    }

    async fn update(&self, id: Uuid, req: UpdateNoteRequest) -> anyhow::Result<Option<NoteRecord>> {
        let mut map = self.notes.write().await;
        let note = match map.get_mut(&id) {
            Some(n) => n,
            None => return Ok(None),
        };
        if let Some(title) = req.title {
            note.title = title;
        }
        if let Some(content) = req.content {
            note.content = content;
        }
        if let Some(tags) = req.tags {
            note.tags = tags;
        }
        note.updated_at = chrono::Utc::now();
        Ok(Some(note.clone()))
    }

    async fn delete(&self, id: Uuid) -> anyhow::Result<bool> {
        Ok(self.notes.write().await.remove(&id).is_some())
    }

    async fn search(&self, query: &str) -> anyhow::Result<Vec<NoteRecord>> {
        self.list(&ListNotesQuery {
            tag: None,
            search: Some(query.to_string()),
        })
        .await
    }
}
