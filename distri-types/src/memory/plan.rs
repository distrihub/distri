//! Plan validation (spec §2.5) and the application of one operation to an item, shared by every
//! [`super::AgentMemoryStore`] so Postgres and the in-memory store agree.

use std::collections::{HashMap, HashSet};

use super::{
    L0_MAX_CHARS, L1_MAX_CHARS, MemoryAuthor, MemoryChange, MemoryItem, MemoryOp, MemoryPlan,
    MemoryStatus, MemoryUri, RULES_FOLDER,
};

/// What a plan is checked against.
pub struct PlanContext<'a> {
    pub space: &'a str,
    pub scope: &'a str,
    /// The folders the agent may use (`MemoryConfig::folders`).
    pub folders: &'a [String],
    /// Current items of the scope, by URI string.
    pub existing: &'a HashMap<String, MemoryItem>,
    /// The ids (`sig:<id>`) a plan may cite; `None` skips the check.
    pub known_signals: Option<&'a HashSet<String>>,
}

/// Every reason the plan cannot be applied (empty: valid).
pub fn validate_plan(plan: &MemoryPlan, ctx: &PlanContext<'_>) -> Vec<String> {
    let mut errors = Vec::new();
    let live = |uri: &MemoryUri| {
        ctx.existing
            .get(&uri.to_string())
            .filter(|i| i.status != MemoryStatus::Archived)
    };
    let created: HashSet<String> = plan
        .ops
        .iter()
        .filter(|op| matches!(op, MemoryOp::Create { .. }))
        .map(|op| op.uri().to_string())
        .collect();
    for (i, op) in plan.ops.iter().enumerate() {
        let uri = op.uri();
        let label = format!("op {} ({} {uri})", i + 1, op.name());
        if uri.space != ctx.space || uri.scope != ctx.scope {
            errors.push(format!(
                "out_of_scope: {label}: outside mem://{}/{}",
                ctx.space, ctx.scope
            ));
            continue;
        }
        if !uri.is_item() {
            errors.push(format!(
                "{label}: needs a slug (mem://space/scope/folder/slug)"
            ));
            continue;
        }
        if uri.folder == RULES_FOLDER {
            errors.push(format!(
                "forbidden: {label}: rules/ is written by the owner only"
            ));
            continue;
        }
        if !ctx.folders.iter().any(|f| *f == uri.folder) {
            errors.push(format!(
                "{label}: unknown folder `{}` (folders: {})",
                uri.folder,
                ctx.folders.join(", ")
            ));
            continue;
        }
        let existing = live(uri);
        if ctx
            .existing
            .get(&uri.to_string())
            .map(|i| i.pinned)
            .unwrap_or(false)
        {
            errors.push(format!(
                "forbidden: {label}: the item is pinned by the owner"
            ));
            continue;
        }
        match op {
            MemoryOp::Create { l0, pinned, .. } => {
                if existing.is_some() {
                    errors.push(format!("exists: {label}: use patch or replace"));
                }
                if l0.trim().is_empty() {
                    errors.push(format!("{label}: l0 is empty"));
                }
                if pinned.is_some() {
                    errors.push(format!("forbidden: {label}: only the owner pins"));
                }
            }
            MemoryOp::Patch { search, .. } => match existing {
                None => errors.push(format!("not_found: {label}: no such item to patch")),
                Some(item) => {
                    let n = if search.is_empty() {
                        0
                    } else {
                        item.body.matches(search.as_str()).count()
                    };
                    if n == 0 {
                        errors.push(format!(
                            "not_found: {label}: the search text is not in the body"
                        ));
                    } else if n > 1 {
                        errors.push(format!(
                            "ambiguous: {label}: the search text occurs {n} times; quote more of it"
                        ));
                    }
                }
            },
            MemoryOp::Replace { pinned, .. } => {
                if existing.is_none() {
                    errors.push(format!("not_found: {label}: no such item to replace"));
                }
                if pinned.is_some() {
                    errors.push(format!("forbidden: {label}: only the owner pins"));
                }
            }
            MemoryOp::Archive {
                replacement,
                reason,
                ..
            } => {
                if existing.is_none() {
                    errors.push(format!("not_found: {label}: no such item to archive"));
                }
                if reason.trim().is_empty() {
                    errors.push(format!(
                        "{label}: an archive needs a reason that says where each fact went"
                    ));
                }
                if let Some(r) = replacement {
                    let in_scope = r.space == ctx.space && r.scope == ctx.scope;
                    if !in_scope || (live(r).is_none() && !created.contains(&r.to_string())) {
                        errors.push(format!(
                            "{label}: replacement {r} does not exist (create it in the same plan)"
                        ));
                    }
                }
            }
        }
        if uri.folder == "experiences"
            && !matches!(op, MemoryOp::Archive { .. })
            && op.evidence().len() < 2
        {
            errors.push(format!(
                "{label}: experiences need at least 2 pieces of evidence (cases or signals)"
            ));
        }
        if let Some(known) = ctx.known_signals {
            for e in op.evidence() {
                if e.starts_with("sig:") && !known.contains(e) {
                    errors.push(format!("{label}: unknown evidence `{e}`"));
                }
            }
        }
    }
    errors
}

fn snapshot(item: &MemoryItem) -> serde_json::Value {
    serde_json::to_value(item).unwrap_or_default()
}

fn check_body(uri: &MemoryUri, body: &str) -> Result<(), String> {
    if body.len() > super::guard::MAX_BODY_BYTES {
        return Err(format!(
            "too_large: {uri}: the body is {} bytes (at most {})",
            body.len(),
            super::guard::MAX_BODY_BYTES
        ));
    }
    Ok(())
}

/// The item's stored text with token-like values removed.
fn scrubbed(mut item: MemoryItem) -> MemoryItem {
    use super::guard::scrub_secrets;
    item.l0 = scrub_secrets(&item.l0);
    item.l1 = item.l1.map(|t| scrub_secrets(&t));
    item.body = scrub_secrets(&item.body);
    item
}

fn check_text(uri: &MemoryUri, l0: &str, l1: Option<&str>) -> Result<(), String> {
    if l0.trim().is_empty() {
        return Err(format!("invalid: {uri}: l0 is empty"));
    }
    if l0.contains('\n') || l0.chars().count() > L0_MAX_CHARS {
        return Err(format!(
            "invalid: {uri}: l0 is one line of at most {L0_MAX_CHARS} characters"
        ));
    }
    if l1
        .map(|t| t.chars().count() > L1_MAX_CHARS)
        .unwrap_or(false)
    {
        return Err(format!(
            "invalid: {uri}: l1 is at most {L1_MAX_CHARS} characters"
        ));
    }
    Ok(())
}

fn default_l1(l1: &Option<String>, body: &str) -> Option<String> {
    l1.clone().filter(|t| !t.trim().is_empty()).or_else(|| {
        (body.chars().count() <= L1_MAX_CHARS && !body.trim().is_empty()).then(|| body.to_string())
    })
}

fn merge_evidence(old: &[String], new: &[String]) -> Vec<String> {
    let mut out = old.to_vec();
    for e in new {
        if !out.contains(e) {
            out.push(e.clone());
        }
    }
    out
}

/// An operation applied: the item as it now is, and its change-log row.
#[derive(Debug)]
pub struct AppliedOp {
    pub item: MemoryItem,
    pub change: MemoryChange,
}

/// Apply `op` to `existing` (the item at `op.uri()`, any status). Errors carry a code prefix
/// (`forbidden:`, `not_found:`, `exists:`, `ambiguous:`).
pub fn apply_op(
    ws: &str,
    existing: Option<&MemoryItem>,
    op: &MemoryOp,
    author: &MemoryAuthor,
    now: i64,
) -> Result<AppliedOp, String> {
    let uri = op.uri();
    let owner = author.is_owner();
    if !uri.is_item() {
        return Err(format!("invalid_uri: {uri}: needs a slug"));
    }
    if !owner && uri.folder == RULES_FOLDER {
        return Err(format!(
            "forbidden: {uri}: rules/ is written by the owner only"
        ));
    }
    if !owner && existing.map(|e| e.pinned).unwrap_or(false) {
        return Err(format!("forbidden: {uri}: the item is pinned by the owner"));
    }
    let live = existing.filter(|e| e.status != MemoryStatus::Archived);
    let agent = author
        .agent()
        .or_else(|| existing.and_then(|e| e.agent.clone()));
    let (item, reason) = match op {
        MemoryOp::Create {
            l0,
            l1,
            body,
            evidence,
            pinned,
            ..
        } => {
            if live.is_some() {
                return Err(format!("exists: {uri}: use patch or replace"));
            }
            if pinned.is_some() && !owner {
                return Err(format!("forbidden: {uri}: only the owner pins"));
            }
            check_text(uri, l0, l1.as_deref())?;
            check_body(uri, body)?;
            (
                MemoryItem {
                    id: existing
                        .map(|e| e.id.clone())
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    workspace_id: ws.to_string(),
                    uri: uri.clone(),
                    agent,
                    l0: l0.trim().to_string(),
                    l1: default_l1(l1, body),
                    body: body.clone(),
                    status: MemoryStatus::Active,
                    pinned: pinned.unwrap_or(false),
                    author: author.clone(),
                    evidence: evidence.clone(),
                    version: existing.map(|e| e.version + 1).unwrap_or(1),
                    used_count: existing.map(|e| e.used_count).unwrap_or(0),
                    last_used_at: existing.and_then(|e| e.last_used_at),
                    created_at: existing.map(|e| e.created_at).unwrap_or(now),
                    updated_at: now,
                },
                None,
            )
        }
        MemoryOp::Patch {
            search,
            replace,
            l0,
            evidence,
            ..
        } => {
            let Some(cur) = live else {
                return Err(format!("not_found: {uri}: no such item to patch"));
            };
            let n = if search.is_empty() {
                0
            } else {
                cur.body.matches(search.as_str()).count()
            };
            if n == 0 {
                return Err(format!(
                    "not_found: {uri}: the search text is not in the body"
                ));
            }
            if n > 1 {
                return Err(format!(
                    "ambiguous: {uri}: the search text occurs {n} times"
                ));
            }
            let mut next = cur.clone();
            next.body = cur.body.replacen(search.as_str(), replace, 1);
            if next.l1.as_deref() == Some(cur.body.as_str()) {
                next.l1 = default_l1(&None, &next.body);
            }
            if let Some(l0) = l0 {
                next.l0 = l0.trim().to_string();
            }
            check_text(uri, &next.l0, next.l1.as_deref())?;
            check_body(uri, &next.body)?;
            next.evidence = merge_evidence(&cur.evidence, evidence);
            next.agent = agent;
            next.author = author.clone();
            next.version += 1;
            next.updated_at = now;
            (next, None)
        }
        MemoryOp::Replace {
            l0,
            l1,
            body,
            evidence,
            pinned,
            ..
        } => {
            let Some(cur) = live else {
                return Err(format!("not_found: {uri}: no such item to replace"));
            };
            if pinned.is_some() && !owner {
                return Err(format!("forbidden: {uri}: only the owner pins"));
            }
            check_text(uri, l0, l1.as_deref())?;
            check_body(uri, body)?;
            let mut next = cur.clone();
            next.l0 = l0.trim().to_string();
            next.l1 = default_l1(l1, body);
            next.body = body.clone();
            if let Some(p) = pinned {
                next.pinned = *p;
            }
            next.evidence = merge_evidence(&cur.evidence, evidence);
            next.agent = agent;
            next.author = author.clone();
            next.version += 1;
            next.updated_at = now;
            (next, None)
        }
        MemoryOp::Archive { reason, .. } => {
            let Some(cur) = live else {
                return Err(format!("not_found: {uri}: no such item to archive"));
            };
            let mut next = cur.clone();
            next.status = MemoryStatus::Archived;
            next.agent = agent;
            next.author = author.clone();
            next.version += 1;
            next.updated_at = now;
            (next, Some(reason.clone()).filter(|r| !r.trim().is_empty()))
        }
    };
    let item = scrubbed(item);
    let change = MemoryChange {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: ws.to_string(),
        uri: uri.clone(),
        agent: item.agent.clone(),
        op: op.name().to_string(),
        before: existing.map(snapshot),
        after: Some(snapshot(&item)),
        author: author.clone(),
        reason,
        evidence: op.evidence().to_vec(),
        created_at: now,
    };
    Ok(AppliedOp { item, change })
}

/// Undo `change` on `current` (the item as it is now): put back the item as it was before the
/// change, or archive it when the change created it.
pub fn restore_op(
    ws: &str,
    current: Option<&MemoryItem>,
    change: &MemoryChange,
    author: &MemoryAuthor,
    now: i64,
) -> Result<AppliedOp, String> {
    let next = match (&change.before, current) {
        (Some(before), cur) => {
            let mut item: MemoryItem = serde_json::from_value(before.clone()).map_err(|e| {
                format!(
                    "invalid: change {} has an unreadable snapshot: {e}",
                    change.id
                )
            })?;
            item.version = cur.map(|c| c.version).unwrap_or(item.version) + 1;
            item.used_count = cur.map(|c| c.used_count).unwrap_or(item.used_count);
            item.last_used_at = cur.and_then(|c| c.last_used_at).or(item.last_used_at);
            item.author = author.clone();
            item.updated_at = now;
            item
        }
        (None, Some(cur)) => {
            let mut item = cur.clone();
            item.status = MemoryStatus::Archived;
            item.author = author.clone();
            item.version += 1;
            item.updated_at = now;
            item
        }
        (None, None) => {
            return Err(format!("not_found: {}: nothing to restore", change.uri));
        }
    };
    let change_row = MemoryChange {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: ws.to_string(),
        uri: change.uri.clone(),
        agent: next.agent.clone(),
        op: "restore".to_string(),
        before: current.map(snapshot),
        after: Some(snapshot(&next)),
        author: author.clone(),
        reason: Some(format!("restore of change {}", change.id)),
        evidence: vec![],
        created_at: now,
    };
    Ok(AppliedOp {
        item: next,
        change: change_row,
    })
}

/// The ids of the signals a plan cites (`sig:<id>` → `<id>`).
pub fn cited_signals(plan: &MemoryPlan) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    for op in &plan.ops {
        for e in op.evidence() {
            if let Some(id) = e.strip_prefix("sig:") {
                *out.entry(id.to_string()).or_insert(0) += 1;
            }
        }
    }
    out
}
