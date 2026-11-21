---
name: code_execution
description: "How to run code in the thread's sandbox with Bash, Read, Write, Edit and save_artifact — Python, shell, files that persist across turns"
---

# Code Execution (Sandbox)

Code runs in the thread's sandbox: a remote Linux container that belongs to this conversation. This is NOT a local shell and NOT the user's machine.

## Tools

The host adds these tools to every run in a sandbox-enabled workspace. Do not declare them in `tools.builtin`; they appear on their own.

1. `Bash({command: "..."})` — runs a shell command. Each call is a fresh process, but the working directory carries over between calls.
2. `Read({file_path: "..."})` — reads a file in the sandbox.
3. `Write({file_path: "...", content: "..."})` — creates or overwrites a file.
4. `Edit({file_path: "...", old_string: "...", new_string: "..."})` — exact string replacement in an existing file. `Read` it first.
5. `save_artifact({path: "..."})` — sends a sandbox file to the user. Relative paths resolve against the current working directory.

The old names `start_shell`, `execute_shell`, `stop_shell` and `distri_execute_code` are accepted in agent files but ignored. There is no session to start or stop.

## Image and Packages

The image is `python:3.12-slim`: `python3` and `pip`, standard Unix tools. Nothing else is preinstalled — no pandas, numpy, matplotlib, requests. Install what you need:

```
Bash: {"command": "pip install -q pandas matplotlib"}
```

Installed packages stay in the sandbox for later turns, so check before reinstalling.

## Key Facts

- **Working directory**: starts at `/workspace`; `HOME=/workspace`.
- **Persistence**: files under `/workspace` persist across turns and across pause/resume of the sandbox. Shell variables and Python interpreter state do NOT persist between `Bash` calls — write scripts to files and run them.
- **Network**: outbound traffic goes through an egress proxy (`HTTP_PROXY`/`HTTPS_PROXY` are set). pip, npm and GitHub are reachable; arbitrary websites may not be. For the web, use the `browser` app (`browser__open`, `browser__read`; search by opening `https://duckduckgo.com/html/?q=<query>`).
- **Output**: only what the command prints comes back. `print()` your results.

## Typical Flow

```
Write: {"file_path": "/workspace/analyze.py", "content": "import pandas as pd\n..."}
Bash:  {"command": "pip install -q pandas && python3 analyze.py"}
Edit:  {"file_path": "/workspace/analyze.py", "old_string": "...", "new_string": "..."}   # fix and re-run
Bash:  {"command": "python3 analyze.py"}
save_artifact: {"path": "chart.png"}
```

For a one-off calculation, `Bash` with `python3 -c "..."` is enough.

## Artifacts

Files in `/workspace` are only visible inside the sandbox. To show the user a chart, CSV or report, call `save_artifact` with its path right after you create it. Don't quote sandbox paths in your final answer; they mean nothing outside the sandbox.

## Tips

- Charts: `matplotlib.use('Agg')`, then `plt.savefig('/workspace/chart.png')` and `save_artifact`.
- Long scripts: write them with `Write`, fix them with `Edit`, run them with `Bash`. Easier to iterate than long `-c` strings.
- Node is not in the image; install it only if you really need it.

## Important: No Host Env Vars

Environment variables from the distri server are NOT available in the sandbox unless passed explicitly.
Do NOT use `os.getenv('GOOGLE_TOKEN')` and expect a value.

If you need an OAuth token for API calls, use `connection_request` via `distri_platform` instead of code.

## When to Use Code vs connection_request

| Need | Use |
|------|-----|
| Call an API with OAuth token | `connection_request` (token auto-injected) |
| Process/transform data | `Bash` + Python in the sandbox |
| Complex multi-step computation | Scripts via `Write` + `Bash` |
| Generate files/charts | Python in the sandbox + `save_artifact` |
| Install and use libraries | `pip install -q ...` in `Bash` |
