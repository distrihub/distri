//! A process-local [`AgentMemoryStore`] (OSS server, CLI, tests). A sqlite twin in
//! `distri-stores` is a follow-up.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use async_trait::async_trait;

use super::plan::{apply_op, restore_op};
use super::*;

#[derive(Default)]
struct State {
    /// (workspace, item uri) → item.
    items: BTreeMap<(String, String), MemoryItem>,
    /// (workspace, folder uri) → folder.
    folders: BTreeMap<(String, String), MemoryFolder>,
    changes: Vec<MemoryChange>,
    signals: Vec<MemorySignal>,
}

impl State {
    /// Recount a folder's items and add `ops` to its pending changes.
    fn touch_folder(&mut self, ws: &str, folder: &MemoryUri, ops: i32, now: i64) {
        let key = (ws.to_string(), folder.to_string());
        let count = self
            .items
            .iter()
            .filter(|((w, _), i)| {
                w == ws && i.uri.folder_uri() == *folder && i.status != MemoryStatus::Archived
            })
            .count() as i32;
        let row = self.folders.entry(key).or_insert_with(|| MemoryFolder {
            uri: folder.clone(),
            agent: None,
            l0: String::new(),
            l1: String::new(),
            item_count: 0,
            pending_changes: 0,
            updated_at: now,
        });
        row.item_count = count;
        row.pending_changes += ops;
        row.updated_at = now;
    }
}

#[derive(Default)]
pub struct InMemoryAgentMemoryStore {
    state: Mutex<State>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn owner() -> MemoryAuthor {
    MemoryAuthor::User {
        user_id: "owner".into(),
    }
}

#[async_trait]
impl AgentMemoryStore for InMemoryAgentMemoryStore {
    async fn spaces(&self, ws: &str) -> anyhow::Result<Vec<MemorySpaceSummary>> {
        let st = self.state.lock().unwrap();
        let mut out: BTreeMap<String, MemorySpaceSummary> = BTreeMap::new();
        let entry = |out: &mut BTreeMap<String, MemorySpaceSummary>, space: &str| {
            out.entry(space.to_string())
                .or_insert_with(|| MemorySpaceSummary {
                    space: space.to_string(),
                    scopes: vec![],
                    items: 0,
                    signals_waiting: 0,
                    agents: vec![],
                    updated_at: 0,
                })
                .space
                .clone()
        };
        for ((w, _), item) in &st.items {
            if w != ws || item.status == MemoryStatus::Archived {
                continue;
            }
            let space = entry(&mut out, &item.uri.space);
            let s = out.get_mut(&space).unwrap();
            s.items += 1;
            s.scopes.push(item.uri.scope.clone());
            s.agents.extend(item.agent.clone());
            s.updated_at = s.updated_at.max(item.updated_at);
        }
        for sig in st.signals.iter().filter(|s| s.workspace_id == ws) {
            let space = entry(&mut out, &sig.space);
            let s = out.get_mut(&space).unwrap();
            if sig.reflected_at.is_none() {
                s.signals_waiting += 1;
            }
            s.scopes.push(sig.scope.clone());
            s.agents.extend(sig.agent.clone());
            s.updated_at = s.updated_at.max(sig.created_at);
        }
        Ok(out
            .into_values()
            .map(|mut s| {
                s.scopes = sorted_unique(s.scopes);
                s.agents = sorted_unique(s.agents);
                s
            })
            .collect())
    }

    async fn index(
        &self,
        ws: &str,
        space: &str,
        scope: &str,
    ) -> anyhow::Result<(Vec<MemoryFolder>, Vec<MemoryItem>)> {
        let st = self.state.lock().unwrap();
        let folders = st
            .folders
            .iter()
            .filter(|((w, _), f)| w == ws && f.uri.space == space && f.uri.scope == scope)
            .map(|(_, f)| f.clone())
            .collect();
        let items = st
            .items
            .iter()
            .filter(|((w, _), i)| {
                w == ws
                    && i.uri.space == space
                    && i.uri.scope == scope
                    && i.status != MemoryStatus::Archived
            })
            .map(|(_, i)| i.clone())
            .collect();
        Ok((folders, items))
    }

    async fn get(&self, ws: &str, uri: &MemoryUri) -> anyhow::Result<Option<MemoryItem>> {
        let st = self.state.lock().unwrap();
        Ok(st.items.get(&(ws.to_string(), uri.to_string())).cloned())
    }

    async fn search(
        &self,
        ws: &str,
        space: &str,
        scope: &str,
        q: &str,
        folder: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryItem>> {
        let terms: Vec<String> = q.split_whitespace().map(|t| t.to_lowercase()).collect();
        let st = self.state.lock().unwrap();
        let mut hits: Vec<MemoryItem> = st
            .items
            .iter()
            .filter(|((w, _), i)| {
                w == ws
                    && i.uri.space == space
                    && i.uri.scope == scope
                    && i.status == MemoryStatus::Active
                    && folder.map(|f| i.uri.folder == f).unwrap_or(true)
            })
            .filter(|(_, i)| {
                let hay = format!(
                    "{}\n{}\n{}",
                    i.l0,
                    i.l1.as_deref().unwrap_or_default(),
                    i.body
                )
                .to_lowercase();
                terms.iter().all(|t| hay.contains(t))
            })
            .map(|(_, i)| i.clone())
            .collect();
        hits.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        hits.truncate(limit);
        Ok(hits)
    }

    async fn apply(
        &self,
        ws: &str,
        plan: &MemoryPlan,
        author: &MemoryAuthor,
    ) -> anyhow::Result<Vec<MemoryChange>> {
        let now = now_ms();
        let mut st = self.state.lock().unwrap();
        // Work on an overlay so a failing op leaves the store as it was.
        let mut overlay: HashMap<String, MemoryItem> = HashMap::new();
        let mut changes = Vec::new();
        for op in &plan.ops {
            let key = op.uri().to_string();
            let current = overlay
                .get(&key)
                .cloned()
                .or_else(|| st.items.get(&(ws.to_string(), key.clone())).cloned());
            let applied =
                apply_op(ws, current.as_ref(), op, author, now).map_err(anyhow::Error::msg)?;
            overlay.insert(key, applied.item);
            changes.push(applied.change);
        }
        let mut per_folder: HashMap<MemoryUri, i32> = HashMap::new();
        for (key, item) in overlay {
            st.items.insert((ws.to_string(), key), item);
        }
        for c in &changes {
            *per_folder.entry(c.uri.folder_uri()).or_insert(0) += 1;
        }
        for (folder, n) in per_folder {
            st.touch_folder(ws, &folder, n, now);
        }
        st.changes.extend(changes.iter().cloned());
        Ok(changes)
    }

    async fn set_folder_summary(
        &self,
        ws: &str,
        folder: &MemoryUri,
        l0: &str,
        l1: &str,
    ) -> anyhow::Result<()> {
        let now = now_ms();
        let mut st = self.state.lock().unwrap();
        let folder = folder.folder_uri();
        st.touch_folder(ws, &folder, 0, now);
        let row = st
            .folders
            .get_mut(&(ws.to_string(), folder.to_string()))
            .expect("touched");
        row.l0 = l0.to_string();
        row.l1 = l1.to_string();
        row.pending_changes = 0;
        Ok(())
    }

    async fn mark_used(&self, ws: &str, uris: &[MemoryUri]) -> anyhow::Result<()> {
        let now = now_ms();
        let mut st = self.state.lock().unwrap();
        for uri in uris {
            if let Some(item) = st.items.get_mut(&(ws.to_string(), uri.to_string())) {
                item.used_count += 1;
                item.last_used_at = Some(now);
            }
        }
        Ok(())
    }

    async fn restore(
        &self,
        ws: &str,
        space: &str,
        change_id: &str,
    ) -> anyhow::Result<MemoryChange> {
        let now = now_ms();
        let mut st = self.state.lock().unwrap();
        let change = st
            .changes
            .iter()
            .find(|c| c.id == change_id && c.workspace_id == ws && c.uri.space == space)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("not_found: change {change_id}"))?;
        let key = (ws.to_string(), change.uri.to_string());
        let current = st.items.get(&key).cloned();
        let applied =
            restore_op(ws, current.as_ref(), &change, &owner(), now).map_err(anyhow::Error::msg)?;
        st.items.insert(key, applied.item);
        st.touch_folder(ws, &change.uri.folder_uri(), 1, now);
        st.changes.push(applied.change.clone());
        Ok(applied.change)
    }

    async fn changes(
        &self,
        ws: &str,
        space: &str,
        scope: Option<&str>,
        limit: usize,
        before: Option<i64>,
    ) -> anyhow::Result<Vec<MemoryChange>> {
        let st = self.state.lock().unwrap();
        let mut out: Vec<MemoryChange> = st
            .changes
            .iter()
            .rev()
            .filter(|c| {
                c.workspace_id == ws
                    && c.uri.space == space
                    && scope.map(|s| c.uri.scope == s).unwrap_or(true)
                    && before.map(|b| c.created_at < b).unwrap_or(true)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out.truncate(limit);
        Ok(out)
    }

    async fn bulk_insert_signals(&self, items: Vec<MemorySignal>) -> anyhow::Result<usize> {
        for sig in &items {
            super::guard::check_signal(sig).map_err(anyhow::Error::msg)?;
        }
        let mut st = self.state.lock().unwrap();
        let mut n = 0;
        for sig in items.into_iter().map(super::guard::scrub_signal) {
            if st.signals.iter().any(|s| s.id == sig.id) {
                continue;
            }
            st.signals.push(sig);
            n += 1;
        }
        Ok(n)
    }

    async fn list_signals(
        &self,
        ws: &str,
        space: &str,
        scope: Option<&str>,
        unreflected: bool,
        limit: usize,
    ) -> anyhow::Result<Vec<MemorySignal>> {
        let st = self.state.lock().unwrap();
        let mut out: Vec<MemorySignal> = st
            .signals
            .iter()
            .filter(|s| {
                s.workspace_id == ws
                    && s.space == space
                    && scope.map(|sc| s.scope == sc).unwrap_or(true)
                    && (!unreflected || s.reflected_at.is_none())
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        out.truncate(limit);
        Ok(out)
    }

    async fn mark_reflected(&self, ws: &str, ids: &[String], at: i64) -> anyhow::Result<()> {
        let mut st = self.state.lock().unwrap();
        for s in st.signals.iter_mut() {
            if s.workspace_id == ws && ids.contains(&s.id) {
                s.reflected_at = Some(at);
            }
        }
        Ok(())
    }
}
