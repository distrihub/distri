---
name: designer
description: "Design and create new agents and skills on the Distri platform. Load when the user wants to build an agent, turn a workflow into a reusable skill, or extend platform capabilities."
tags: [platform/agents, platform/skills, management, builtin]
---

# Designer

Use this skill when you need to **create more Distri** — new agents for specialized roles, or new skills for reusable workflows. Pairs with `distri_platform` (which covers the API surface) — this skill focuses on the *design* decisions and *good patterns*.

## When to use

- User asks to create or design a new **agent** (e.g. "build me a sales agent")
- User asks to **save a workflow as a skill** after a successful task
- User wants to **turn a multi-step process into a template** future runs can reuse
- User wants to **modify an existing agent** (instructions, tools, connections)

For the raw API reference (endpoints, bodies, examples), also load `distri_platform`.

---

# Part 1 — Designing Agents

## Methodology

**1. Understand the role.** Ask yourself:
- What *decisions* will this agent make?
- What *data* does it read and produce?
- Who calls it — the user directly, or another agent?
- What's the *one sentence* summary of its job? If you can't state it in one sentence, split it into multiple agents.

**2. Pick capabilities.** Based on the role:
- **Tools** — what built-in tools (`search`, `browsr_scrape`, `distri_request`, shell tools) does it need?
- **Sub-agents** — can it delegate to specialists (`coder`, `explore`, `plan`)? Prefer delegation over stuffing the parent agent with every tool.
- **Skills** — which skills should it be allowed to load? Use `[{"id": "*", "name": "*"}]` for "all" during prototyping; narrow once the agent is stable.
- **Connections** — does it talk to external services (Google, Slack, GitHub)? OAuth connections are attached to the user, not the agent — the agent references them by ID at call time.

**3. Write focused instructions.** The `instructions` field is the agent's system prompt. Good prompts:
- Start with a clear `# ROLE` statement (one paragraph, what you are and aren't)
- Include `# TASK` with `{{task}}` placeholder for the incoming user request
- List capabilities concretely — don't say "you have tools", say "you use `search` for web lookups and `distri_request` for platform calls"
- State rules as imperatives ("Always call `final` when done" > "You should probably call `final`")
- Keep it short. 80–150 lines is a healthy range. If it's longer, the agent probably has too many responsibilities.

**4. Single-responsibility.** Each agent should have *one* clear job. Prefer multiple specialized agents (sales-intake, sales-followup) over one monolithic agent (sales). Orchestrator agents like `distri` delegate to specialists — they don't do the specialist work themselves.

## Agent TOML Skeleton

```toml
---
name = "my-agent"                     # lowercase, dashes or underscores OK
description = "What this agent does in one sentence"
max_iterations = 25                   # budget; 10-15 for narrow, 25-60 for broad
tool_format = "provider"              # native tool calling; keep this default
include_scratchpad = true             # accumulates agent's scratchpad between steps
sub_agents = ["coder"]                # delegates allowed

[strategy]
reasoning_depth = "standard"          # "shallow" | "standard" | "deep"

[strategy.execution_mode]
type = "tools"                         # tool-based execution

[tools]
builtin = ["final", "search", "distri_request", "load_skill", "tool_search"]
external = []                          # external tool names (if any)

[[available_skills]]
id = "*"
name = "*"
---

# ROLE
...
```

## Creating via API

```json
distri_request({
  "method": "POST",
  "path": "/agents",
  "body": {
    "agent_type": "standard_agent",
    "name": "sales-intake",
    "description": "Qualifies inbound leads and creates CRM records",
    "instructions": "# ROLE\nYou are a sales intake specialist...\n\n# TASK\n{{task}}\n\n...",
    "sub_agents": ["coder"],
    "tool_format": "provider",
    "tool_delivery_mode": "tool_search",
    "max_iterations": 25,
    "enable_todos": true,
    "available_skills": [{"id": "*", "name": "*"}]
  }
})
```

See `distri_platform` for the full schema and edit/delete endpoints.

## Multi-Agent Systems

When a task spans multiple roles (e.g. "build me a CTO assistant"):

1. **Map roles → agents.** CTO → one agent. Engineering Manager → another. Each has a single clear responsibility.
2. **Identify shared resources.** Which OAuth connections (GitHub, Notion, Slack) do all agents need? Which skills are common?
3. **Pick a hub.** Usually an orchestrator agent (or `distri` itself) delegates to the specialists via `call_agent`.
4. **Design communication.** Which agents can delegate to which? (sub_agents list) Keep the graph shallow — deep call chains are hard to debug.
5. **Build incrementally.** Create one agent, test it. Add the next. Don't ship all five at once.

---

# Part 2 — Designing Skills

## When to extract a skill

After completing a non-trivial task (typically via `call_coder`), ask:

- **Is the *workflow* reusable?** → Save as a skill (API integration template, data pipeline, deployment recipe).
- **Is it a *fact* worth remembering?** → Save as a workspace note, not a skill.
- **Was it a one-off computation?** → Don't save.

Rule of thumb: if you'd run the same sequence of tool calls + the same kind of prompt to tackle a similar task next week, it's a skill.

## Good skill structure

Skills are **pure markdown files** with YAML frontmatter:

```markdown
---
name: my-skill
description: "One-line hook — what this skill lets the agent do"
tags: [domain, category]
---

# Skill Name

One-paragraph overview.

## When to use
- Trigger 1
- Trigger 2

## Steps
1. Do X (concrete: which tool, what input)
2. Do Y
3. Do Z

## Example
Concrete example of input and expected output.

## Gotchas
- Edge cases, rate limits, auth quirks
```

Skills read best when they're **playbook-shaped**: clear triggers, ordered steps, concrete examples. Avoid philosophy — the agent already has a general prompt; skills fill in *how* for a specific situation.

Whenever a step in a skill requires real code (API call, data parsing, file transformation), the skill instructs the agent to delegate to `distri_runner`. The skill itself stays markdown.

## Creating via API

```json
distri_request({
  "method": "POST",
  "path": "/skills",
  "body": {
    "name": "slack-daily-digest",
    "description": "Posts a daily summary to a Slack channel",
    "content": "# Slack Daily Digest\n\n...",
    "tags": ["slack", "daily", "connections/slack"],
    "path": "connections/slack",
    "is_public": false
  }
}
```

### Hierarchical paths

Organise with the `path` field so skills are discoverable:

- **`platform/*`** — Core platform capabilities (`platform/agents`, `platform/skills`)
- **`connections/*`** — Per-service integrations (`connections/slack`, `connections/github`)
- **`custom/*`** — User domain skills (`custom/finance`, `custom/devops`)

### Code vs skill

Skills are **pure markdown** — instructions, triggers, examples, gotchas. They don't carry executable code. When a skill needs deterministic work done (call an API, parse a file, run a computation), the agent reading the skill calls `distri_runner` (sandboxed Python + shell) to run that code. Keep skill markdown focused on the *when* and *what*; let `distri_runner` handle the *how* for anything that's real code.

---

# Shared Principles

- **Be specific.** "Fetches users from the API" beats "handles user data".
- **Name with intent.** `slack-daily-digest` not `slack-thing-v2`.
- **Test before shipping.** Create → run through one end-to-end task → refine instructions/steps → only then suggest it for wider use.
- **Prefer delegation.** An orchestrator agent that calls three specialist agents is cleaner than one agent trying to do everything.
- **Keep feedback loops short.** When an agent or skill fails, read the spans/events, narrow the fix, iterate.

---

# Checklist before saying "done"

When creating an agent or skill, verify:

- [ ] Name is unique and follows convention (dashes/underscores, lowercase)
- [ ] Description reads as a single sentence in a list view
- [ ] For agents: instructions end with "Always call `final` when done"
- [ ] For skills: `tags` and `path` are set so it's discoverable
- [ ] You've tested it with one realistic task before recommending it to the user
