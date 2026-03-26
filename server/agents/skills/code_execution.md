---
name = "code_execution"
description = "How to use the browsr shell for code execution — Python, bash, JS in sandboxed containers"
---

# Code Execution (Browsr Shell)

Code runs in remote sandboxed containers via browsr shell. This is NOT a local shell.

## Tools

1. `start_shell({language: "python"})` — creates a Python REPL session
2. `execute_shell({command: "..."})` — runs code in the active session
3. `stop_shell()` — destroys the session

## Pre-installed Packages

The custom Python image includes these packages — **no pip install needed**:

| Package | Purpose |
|---------|---------|
| `requests` | HTTP requests |
| `beautifulsoup4`, `lxml` | HTML/XML parsing |
| `pandas`, `numpy`, `scipy` | Data processing |
| `yfinance` | Stock market data |
| `matplotlib` | Charts and plots |
| `openpyxl` | Excel file read/write |

The custom Node image includes: `axios`, `cheerio`, `lodash`, `csv-parse`, `json2csv`.

For packages NOT in this list, install via subprocess:
```python
import subprocess, sys
subprocess.check_call([sys.executable, '-m', 'pip', 'install', '-q', 'package_name'])
```

## Key Facts

- **Language**: Python 3 by default. Also supports `bash` and `javascript`.
- **Remote container**: No access to host env vars, files, or network configs.
- **Network**: Full outbound internet access (HTTP, HTTPS, DNS).
- **Filesystem**: `/workspace` is the working directory. Files persist within the session.
- **Memory**: Default 256MB, configurable via `memory_mb` parameter.
- **Timeout**: Default 300s session timeout, 30s per command. Configurable.

## Artifact Storage

Files saved to `/workspace/` persist within the session. To save artifacts that survive across sessions, use the `artifact_tool` or return data via `final`.

## Language-specific Tips

### Python
- Use `subprocess.run()` for bash commands from Python shell
- `print()` everything — only stdout is captured
- Variables persist between `execute_shell` calls in the same session
- For charts: `plt.savefig('/workspace/chart.png')` then return via artifact

### Node.js
- Global npm packages are available via `require()`
- Use `console.log()` for output

### Bash
- Full bash shell with standard Unix tools
- `curl`, `wget`, `jq` available

## Important: No Host Env Vars

Environment variables from the distri server are NOT available in the shell.
Do NOT use `os.getenv('GOOGLE_TOKEN')` — it will be None.

If you need an OAuth token for API calls, use `connection_request` via `distri_platform` instead of code execution.

## When to Use Code vs connection_request

| Need | Use |
|------|-----|
| Call an API with OAuth token | `connection_request` (token auto-injected) |
| Process/transform data | `call_code` with Python |
| Complex multi-step computation | `call_code` with Python |
| Generate files/charts | `call_code` with Python |
| Install and use libraries | `call_code` (pip install via subprocess) |
