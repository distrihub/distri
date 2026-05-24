//! Authorization route rules contributed by distri-server.
//!
//! Phase 5 of the auth system implementation plan. distri-cloud's auth
//! middleware composes these with `cloud_route_rules()` to form the full
//! `RouteAuthTable`. distri-server itself does not enforce — the trait
//! default `Authorize::authorize` returns `Ok(())` in standalone mode, so
//! these rules only become load-bearing when an embedder (cloud) wires the
//! middleware in.
//!
//! Paths are declared WITHOUT a `/v1` prefix because distri-server mounts
//! under whatever scope its embedder chooses. The cloud-side composer
//! prepends `/v1` before lookup.

use distri_types::authz::{Action, HttpMethod};
use distri_types::{ResourceKind, RouteRule};

fn p(
    path: &str,
    methods: &[HttpMethod],
    kind: ResourceKind,
    action: Action,
) -> RouteRule {
    RouteRule::perm(path, methods.iter().copied(), kind, action)
}

/// Auth rules for every route registered by [`crate::routes::distri`].
///
/// Cloud should call this and prepend its mount prefix when composing the
/// final `RouteAuthTable`.
pub fn distri_server_route_rules() -> Vec<RouteRule> {
    vec![
        // ── Agent CRUD + a2a entry ────────────────────────────────────────
        // Agent card is intentionally public per a2a spec.
        RouteRule::public("/agents/{name}/.well-known/agent.json"),
        p("/agents", &[HttpMethod::Get], ResourceKind::Agent, Action::Read),
        p("/agents", &[HttpMethod::Post], ResourceKind::Agent, Action::Manage),
        p(
            "/agents/{id:.*}/validate",
            &[HttpMethod::Get],
            ResourceKind::Agent,
            Action::Read,
        ),
        p(
            "/agents/{id:.*}/complete-tool",
            &[HttpMethod::Post],
            ResourceKind::Completion,
            Action::Execute,
        ),
        p(
            "/agents/{id:.*}/dag",
            &[HttpMethod::Get],
            ResourceKind::Agent,
            Action::Read,
        ),
        p(
            "/agents/{id:.*}",
            &[HttpMethod::Get],
            ResourceKind::Agent,
            Action::Read,
        ),
        // A2A dispatch — JSON-RPC POST endpoint.
        p(
            "/agents/{id:.*}",
            &[HttpMethod::Post],
            ResourceKind::Completion,
            Action::Execute,
        ),
        p(
            "/agents/{id:.*}",
            &[HttpMethod::Put, HttpMethod::Delete],
            ResourceKind::Agent,
            Action::Manage,
        ),

        // ── Hooks ─────────────────────────────────────────────────────────
        p(
            "/event/hooks",
            &[HttpMethod::Post],
            ResourceKind::Completion,
            Action::Execute,
        ),

        // ── Tasks / threads / tools (transactional, Completion:Execute) ──
        p(
            "/tasks",
            &[HttpMethod::Get],
            ResourceKind::Completion,
            Action::Execute,
        ),
        p("/tools", &[HttpMethod::Get], ResourceKind::Skill, Action::Read),
        p(
            "/threads",
            &[HttpMethod::Get],
            ResourceKind::Completion,
            Action::Execute,
        ),
        p(
            "/threads/agents",
            &[HttpMethod::Get],
            ResourceKind::Observability,
            Action::Read,
        ),
        p(
            "/threads/{thread_id}/messages",
            &[HttpMethod::Get],
            ResourceKind::Completion,
            Action::Execute,
        ),
        p(
            "/threads/{thread_id}",
            &[HttpMethod::Get, HttpMethod::Put, HttpMethod::Delete],
            ResourceKind::Completion,
            Action::Execute,
        ),
        p(
            "/threads/{thread_id}/messages/{message_id}/read",
            &[HttpMethod::Get, HttpMethod::Post],
            ResourceKind::Completion,
            Action::Execute,
        ),
        p(
            "/threads/{thread_id}/read-status",
            &[HttpMethod::Get],
            ResourceKind::Completion,
            Action::Execute,
        ),
        p(
            "/threads/{thread_id}/messages/{message_id}/vote",
            &[HttpMethod::Get, HttpMethod::Post, HttpMethod::Delete],
            ResourceKind::Completion,
            Action::Execute,
        ),
        p(
            "/threads/{thread_id}/messages/{message_id}/votes",
            &[HttpMethod::Get],
            ResourceKind::Observability,
            Action::Read,
        ),

        // ── Schema + meta ─────────────────────────────────────────────────
        RouteRule::public("/schema/agent"),
        p(
            "/device",
            &[HttpMethod::Get],
            ResourceKind::Observability,
            Action::Read,
        ),
        p(
            "/home/stats",
            &[HttpMethod::Get],
            ResourceKind::Observability,
            Action::Read,
        ),

        // ── Files / sessions / artifacts (transactional) ──────────────────
        p(
            "/files/*",
            &[HttpMethod::Any],
            ResourceKind::Completion,
            Action::Execute,
        ),
        p(
            "/sessions/*",
            &[HttpMethod::Any],
            ResourceKind::Completion,
            Action::Execute,
        ),
        p(
            "/artifacts/*",
            &[HttpMethod::Any],
            ResourceKind::Completion,
            Action::Execute,
        ),

        // ── Build / browser / llm / proxy ─────────────────────────────────
        p(
            "/build",
            &[HttpMethod::Post],
            ResourceKind::Workflow,
            Action::Manage,
        ),
        p(
            "/browser/session",
            &[HttpMethod::Post],
            ResourceKind::Completion,
            Action::Execute,
        ),
        p(
            "/llm/execute",
            &[HttpMethod::Post],
            ResourceKind::Completion,
            Action::Execute,
        ),
        p(
            "/request",
            &[HttpMethod::Post],
            ResourceKind::Completion,
            Action::Execute,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use distri_types::RouteAuthTable;

    #[test]
    fn rules_build_and_have_coverage() {
        let rules = distri_server_route_rules();
        assert!(rules.len() > 20, "expected >20 rules, got {}", rules.len());
    }

    #[test]
    fn a2a_dispatch_requires_completion_execute() {
        let table = RouteAuthTable::from_rules(distri_server_route_rules());
        let rule = table
            .lookup("/agents/foo", HttpMethod::Post)
            .expect("a2a dispatch must have a rule");
        assert!(matches!(
            rule.requirement,
            distri_types::RouteRequirement::Permission {
                kind: ResourceKind::Completion,
                action: Action::Execute,
            }
        ));
    }

    #[test]
    fn agent_card_is_public() {
        let table = RouteAuthTable::from_rules(distri_server_route_rules());
        let rule = table
            .lookup("/agents/foo/.well-known/agent.json", HttpMethod::Get)
            .expect("agent card must have a rule");
        assert!(rule.requirement.is_public());
    }

    #[test]
    fn agent_crud_distinct_from_a2a() {
        let table = RouteAuthTable::from_rules(distri_server_route_rules());
        let r = table.lookup("/agents", HttpMethod::Get).unwrap();
        assert!(matches!(
            r.requirement,
            distri_types::RouteRequirement::Permission {
                kind: ResourceKind::Agent,
                action: Action::Read,
            }
        ));
        let r = table.lookup("/agents", HttpMethod::Post).unwrap();
        assert!(matches!(
            r.requirement,
            distri_types::RouteRequirement::Permission {
                kind: ResourceKind::Agent,
                action: Action::Manage,
            }
        ));
    }
}
