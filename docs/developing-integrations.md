# Developing distri Integrations (LLM-Friendly Guide)

This guide packages the learnings from the main `distri` repository so an LLM pair-programmer can iteratively build plugins inside this workspace. Keep it open in your chat when scoping tasks.

---

## 1. Understand the Runtime Contract

- **Plugin shape** – export a default `DistriPlugin` with `integrations` and `workflows` arrays, matching `distri-plugin-executor/src/executors/ts_executor/modules/base.ts`.
- **Tools** – create with `createTool({ name, description, parameters, execute })`. Each tool runs in an async context that mirrors the Rust executor (`context.session_id`, `context.secrets`, `context.auth_session`).
- **Workflows** – define a `DapWorkflow` with `execute(params, context)` and optional `parameters`/`examples`. Runtime utilities inject `callAgent`/`callTool` just like the production executor.
- **Call funnel** – in the Rust runtime, `callTool` and `callAgent` are registered on `globalThis.rustyscript`. The shared runtime module re-implements these entry points so you can test without Rust.

> 🔎 Reference: [`notes/typescript-plugins.md`](../../distri/notes/typescript-plugins.md) explains the TypeScript loader, agent config rules, and example workflows.

---

## 2. Checklist for a New Integration

1. **Scaffold** a folder under `plugins/your-plugin` and add `mod.ts`, `README.md`, `distri.toml`.
2. **Model dependencies** using Deno-compatible imports (HTTP or relative). Avoid Node-only packages.
3. **Implement tools** with clear descriptions, JSON Schema-style parameter definitions, and context-aware secret handling.
4. **Log meaningfully** but tersely; prefer one-line summaries that help a human trace execution.
5. **Document** required auth/secrets in the plugin README and `distri.toml`.
6. **Register & test** locally using the runtime helpers.
7. **Review** for parity with the expectations of the Rust executor (e.g. optional `requiresAuth`, metadata fields, error messages).

---

## 3. Prompt Templates for LLM Pairing

Use these snippets when guiding an LLM through a task.

### a. Build a Tool
```
Goal: create a <service> integration.
Constraints:
- Export a DistriPlugin from plugins/<service>/mod.ts
- Use createTool from jsr:@distri/runtime@0.1.0
- Read secrets from context.secrets (list keys)
- Provide JSON Schema parameters and describe required authentication
Deliver: TypeScript implementation + README + distri.toml metadata.
```

### b. Add a Workflow
```
Goal: author a workflow that consumes tools X, Y.
Requirements:
- New package under plugins/<name>/
- DapWorkflow with execute(params, context)
- Use callTool/callAgent from jsr:@distri/runtime@0.1.0
- Add at least one example entry and document dependencies in README/distri.toml
```

### c. Validate Before Handoff
```
Before final answer:
- registerPlugin + callTool snippet compiled?
- docs mention required secrets/auth?
- distri.toml lists tools and tags?
```

---

## 4. Testing with the Local Runtime

```ts
import { registerPlugin, callTool, clearRuntime } from "jsr:@distri/runtime@0.1.0";
import plugin from "./plugins/notion/mod.ts";

clearRuntime();
registerPlugin(plugin);

const result = await callTool({
  integration: "notion",
  tool_name: "search_pages",
  input: { query: "roadmap" },
  context: { secrets: { NOTION_API_KEY: "secret" } },
});
console.log(result);
```

- `registerAgentHandler()` lets you stub agent calls for workflow tests.
- `clearRuntime()` resets between test runs to avoid tool alias collisions.
- The runtime normalises tool names (`integration_tool`, `integration.tool`, `tool`) similar to the production executor.

---

## 5. Metadata & Packaging Tips

- **`distri.toml`** – capture tags, tool lists, and auth hints; it mirrors the manifest structure consumed by the Rust side.
- **Versioning** – start at `1.0.0` but bump when behaviour changes; tooling reads this for registry publishing.
- **Auth fields** – use consistent naming (`provider`, `type`, `scopes`). Values map directly to the runtime’s auth provider config.

---

## 6. Common Pitfalls (and Fixes)

| Pitfall | Detection | Fix |
| --- | --- | --- |
| Missing secrets | Tool throws before making API call | Document expected secret keys; provide helpful error messages |
| Duplicate tool names | Unexpected tool invoked | Use `integration` when calling tools or namespace your plugin when registering |
| Workflow agent calls fail | `callAgent` throws | Register a stub agent handler during local tests |
| Non-Deno dependencies | Import error | Prefer HTTP imports or rewrite using standard fetch APIs |

---

## 7. Handoff Notes

When you’re ready to move code back into `blinklogic/distri`:

1. Copy the package into the main repo’s plugin directory.
2. Replace the local runtime import with `https://distri.dev/base.ts` (or the internal path used by the executor).
3. Wire authentication providers into the registry manifests as needed.
4. Add integration tests or Rust-side fixtures where appropriate.

Keeping these steps in mind ensures a smooth transition from prototype to production plugin.
