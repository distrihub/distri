//! Canonical route definitions for distri-server — the **single source of
//! truth** for every path distri-server registers and the access weight each
//! one carries.
//!
//! The [`define_routes!`] macro takes one line per route — variant, path, and
//! per-method [`Access`] — and generates:
//!
//! - `Route` enum (type-safe identity, e.g. [`Route::Agents`])
//! - [`Route::path`] / [`Route::methods`] — `const fn`s, co-located so a path,
//!   its methods, and their access weights can never drift apart
//! - [`Route::ALL`] — every variant, for exhaustive iteration
//! - [`DISTRI_SERVER_ROUTES`] — `&[(path, &[(method, Access)])]` for embedders
//!
//! `routes::distri` registers handlers with `web::resource(Route::X.path())`
//! — it never writes a path string inline.
//!
//! ## Access is a *lightweight* hint, not an authorization model
//!
//! distri-server is auth-agnostic. [`Access`] only says what an endpoint
//! *does* — read, write, manage (create/configure/delete), execute (run), or
//! it's public. It carries **no** notion of resource kinds, roles, or
//! permissions. An embedder (distri-cloud) maps `Access` onto its own
//! authorization actions and pins the mapping with a coverage test, so adding
//! a route here forces the embedder to classify it.
//!
//! Paths are relative (no `/v1` prefix); embedders prepend their mount scope.

/// Lightweight operation semantics for a single route+method. NOT an
/// authorization model — embedders map these onto their own action taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    /// No authentication required (e.g. the a2a agent card).
    Public,
    /// Read-only.
    Read,
    /// Mutates an existing resource (create/update of owned data).
    Write,
    /// Administrative: delete / configure / control-plane mutation.
    Manage,
    /// Runs or invokes — the execution surface reachable by a run token
    /// (a2a dispatch, llm execute, sandbox files/sessions/artifacts, …).
    Execute,
}

macro_rules! define_routes {
    (
        $(
            $(#[$meta:meta])*
            $variant:ident => $path:literal { $( $method:ident : $access:ident ),+ $(,)? }
        ),+ $(,)?
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

            /// `(method, access)` pairs this path serves.
            pub const fn methods(self) -> &'static [(&'static str, Access)] {
                match self {
                    $( Route::$variant => &[ $( (stringify!($method), Access::$access) ),+ ] ),+
                }
            }
        }

        /// `(path, &[(method, access)])` for every route — derived from the
        /// same macro input as [`Route`], so registration and catalog cannot
        /// drift.
        pub const DISTRI_SERVER_ROUTES: &[(&str, &[(&str, Access)])] = &[
            $( ($path, &[ $( (stringify!($method), Access::$access) ),+ ]) ),+
        ];
    };
}

define_routes! {
    // ── Agents (the embed-critical surface: a2a dispatch + CRUD) ────────────
    /// Agent card — a2a discovery doc; intentionally public.
    AgentCard         => "/agents/{agent_name}/.well-known/agent.json" { GET: Public },
    /// Lightweight agent-card list — the client/external surface. Returns only
    /// discovery metadata (name, description, version, icon, skills), never the
    /// system prompt / tools / model config that the full `/agents` list exposes.
    AgentCards        => "/agents/cards" { GET: Read },
    Agents            => "/agents" { GET: Read, POST: Write },
    AgentValidate     => "/agents/{id:.*}/validate" { GET: Execute },
    AgentCompleteTool => "/agents/{id:.*}/complete-tool" { POST: Execute },
    AgentDag          => "/agents/{id:.*}/dag" { GET: Execute },
    /// a2a JSON-RPC dispatch (POST=run) + agent definition CRUD.
    AgentDispatch     => "/agents/{id:.*}" { GET: Read, POST: Execute, PUT: Write, DELETE: Manage },

    // ── Hooks / tasks / tools (run surface) ─────────────────────────────────
    EventHooks        => "/event/hooks" { POST: Execute },
    Tasks             => "/tasks" { GET: Execute },
    TaskCompact       => "/tasks/{task_id}/compact" { POST: Execute },
    Tools             => "/tools" { GET: Execute },

    // ── Threads + messages (run surface) ────────────────────────────────────
    Threads           => "/threads" { GET: Execute },
    ThreadsAgents     => "/threads/agents" { GET: Execute },
    ThreadMessages    => "/threads/{thread_id}/messages" { GET: Execute },
    Thread            => "/threads/{thread_id}" { GET: Execute, PUT: Execute, DELETE: Execute },
    ThreadMessageRead => "/threads/{thread_id}/messages/{message_id}/read" { GET: Execute, POST: Execute },
    ThreadReadStatus  => "/threads/{thread_id}/read-status" { GET: Execute },
    ThreadMessageVote => "/threads/{thread_id}/messages/{message_id}/vote" { GET: Execute, POST: Execute, DELETE: Execute },
    ThreadMessageVotes=> "/threads/{thread_id}/messages/{message_id}/votes" { GET: Execute },

    // ── Schema / meta (read-only) ───────────────────────────────────────────
    SchemaAgent       => "/schema/agent" { GET: Read },
    Device            => "/device" { GET: Read },
    HomeStats         => "/home/stats" { GET: Read },

    // ── Sub-scopes — the sandbox/run surface; submodules register the leaves,
    //    an embedder's `<base>/*` rule covers the subtree ────────────────────
    FilesScope        => "/files" { GET: Execute, POST: Execute, PUT: Execute, DELETE: Execute },
    SessionsScope     => "/sessions" { GET: Execute, POST: Execute, PUT: Execute, DELETE: Execute },
    ArtifactsScope    => "/artifacts" { GET: Execute, POST: Execute, PUT: Execute, DELETE: Execute },

    // ── Build / browser / llm / proxy (run surface) ─────────────────────────
    Build             => "/build" { POST: Execute },
    BrowserSession    => "/browser/session" { POST: Execute },
    LlmExecute        => "/llm/execute" { POST: Execute },
    Request           => "/request" { POST: Execute },
}

/// Flat `(path, method, access)` over every route+method — the helper an
/// embedder uses to drive its authorization-coverage check.
pub fn route_access() -> impl Iterator<Item = (&'static str, &'static str, Access)> {
    DISTRI_SERVER_ROUTES
        .iter()
        .flat_map(|(path, methods)| methods.iter().map(move |(m, a)| (*path, *m, *a)))
}
