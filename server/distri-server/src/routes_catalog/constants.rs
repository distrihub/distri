//! Canonical route definitions for distri-server — the **single source of
//! truth** for every path distri-server registers.
//!
//! The [`define_routes!`] macro takes one line per route — variant, path, and
//! the HTTP methods that path serves — and generates:
//!
//! - `Route` enum (type-safe identity, e.g. [`Route::Agents`])
//! - [`Route::path`] / [`Route::methods`] — `const fn`s, co-located so a
//!   path and its method set can never drift apart
//! - [`Route::ALL`] — every variant, for exhaustive iteration
//! - [`DISTRI_SERVER_ROUTES`] — a plain `&[(path, methods)]` slice that
//!   embedders (distri-cloud) consume to check authorization coverage
//!
//! `routes::distri` registers handlers with `web::resource(Route::X.path())`
//! — it never writes a path string inline. Add a route here and registration
//! + catalog stay in lockstep.
//!
//! Paths are relative (no `/v1` prefix); embedders prepend their mount scope.
//! No authorization concepts live here — this is pure routing data.

macro_rules! define_routes {
    (
        $( $(#[$meta:meta])* $variant:ident => $path:literal [ $($method:ident),+ $(,)? ] ),+ $(,)?
    ) => {
        /// Identity of a route registered by `routes::distri`.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Route {
            $( $(#[$meta])* $variant ),+
        }

        impl Route {
            /// Every route variant, in declaration order.
            pub const ALL: &'static [Route] = &[ $( Route::$variant ),+ ];

            /// The actix path pattern this route registers under.
            pub const fn path(self) -> &'static str {
                match self {
                    $( Route::$variant => $path ),+
                }
            }

            /// HTTP methods this path serves (as uppercase strings).
            pub const fn methods(self) -> &'static [&'static str] {
                match self {
                    $( Route::$variant => &[ $( stringify!($method) ),+ ] ),+
                }
            }
        }

        /// `(path, methods)` for every route — derived from the same macro
        /// input as the [`Route`] enum, so it cannot drift from registration.
        pub const DISTRI_SERVER_ROUTES: &[(&str, &[&str])] = &[
            $( ($path, &[ $( stringify!($method) ),+ ]) ),+
        ];
    };
}

define_routes! {
    // ── Agents (the embed-critical surface: a2a dispatch + CRUD) ────────────
    /// Agent card — a2a spec discovery doc, intentionally public.
    AgentCard         => "/agents/{agent_name}/.well-known/agent.json" [GET],
    Agents            => "/agents" [GET, POST],
    AgentValidate     => "/agents/{id:.*}/validate" [GET],
    AgentCompleteTool => "/agents/{id:.*}/complete-tool" [POST],
    AgentDag          => "/agents/{id:.*}/dag" [GET],
    /// a2a JSON-RPC dispatch (POST) + agent definition CRUD (GET/PUT/DELETE).
    /// The most important embedded route.
    AgentDispatch     => "/agents/{id:.*}" [GET, POST, PUT, DELETE],

    // ── Hooks / tasks / tools ───────────────────────────────────────────────
    EventHooks        => "/event/hooks" [POST],
    Tasks             => "/tasks" [GET],
    Tools             => "/tools" [GET],

    // ── Threads + messages ──────────────────────────────────────────────────
    Threads           => "/threads" [GET],
    ThreadsAgents     => "/threads/agents" [GET],
    ThreadMessages    => "/threads/{thread_id}/messages" [GET],
    Thread            => "/threads/{thread_id}" [GET, PUT, DELETE],
    ThreadMessageRead => "/threads/{thread_id}/messages/{message_id}/read" [GET, POST],
    ThreadReadStatus  => "/threads/{thread_id}/read-status" [GET],
    ThreadMessageVote => "/threads/{thread_id}/messages/{message_id}/vote" [GET, POST, DELETE],
    ThreadMessageVotes=> "/threads/{thread_id}/messages/{message_id}/votes" [GET],

    // ── Schema / meta ───────────────────────────────────────────────────────
    SchemaAgent       => "/schema/agent" [GET],
    Device            => "/device" [GET],
    HomeStats         => "/home/stats" [GET],

    // ── Sub-scopes (submodules register the leaves under these bases; an
    //    embedder's `<base>/*` rule covers the whole subtree) ────────────────
    FilesScope        => "/files" [GET, POST, PUT, DELETE],
    SessionsScope     => "/sessions" [GET, POST, PUT, DELETE],
    ArtifactsScope    => "/artifacts" [GET, POST, PUT, DELETE],

    // ── Build / browser / llm / proxy ───────────────────────────────────────
    Build             => "/build" [POST],
    BrowserSession    => "/browser/session" [POST],
    LlmExecute        => "/llm/execute" [POST],
    Request           => "/request" [POST],
}
