# Default Agents

## distri — Master orchestrator

```bash
distri-server run distri --task "Find the latest SpaceX launch date and calculate how many days from now"
```

## explore — Read-only web lookups

```bash
distri-server run explore --task "What are the top 3 programming languages in 2026?"
```

## plan — Read-only implementation plans

```bash
distri-server run plan --task "Plan an agent that monitors stock prices and sends alerts when they cross a threshold"
```

## distri_runner / distri_browser_runner — Code and data runners

`distri_runner` runs code on the CLI user's machine; `distri_browser_runner` runs JavaScript in the
user's browser tab.

## Web access

Agents read the web through the `browser` MCP app (obscura in the thread's sandbox), which
distri-cloud serves in every workspace:

```toml
[[tools.mcp]]
server = "browser"
```

It gives `browser__open`, `browser__navigate`, `browser__read`, `browser__click`, `browser__type`,
`browser__press`, `browser__scroll`, `browser__screenshot` and `browser__close`. To search the web,
open a search page (`https://duckduckgo.com/html/?q=<query>`) and read it. The standalone server has
no `browser` app; the entry is skipped there.
