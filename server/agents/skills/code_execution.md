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

## Key Facts

- **Language**: Python 3 by default. Also supports `bash` and `javascript`.
- **Remote container**: No access to host env vars, files, or network configs.
- **Pre-installed packages**: Standard library only. No `requests`, `pandas`, etc.
- **To install packages**: Use `subprocess` in Python, NOT `pip install` directly:
  ```python
  import subprocess, sys
  subprocess.check_call([sys.executable, '-m', 'pip', 'install', 'requests'])
  ```
- **To run bash commands from Python shell**: Use `subprocess.run()`:
  ```python
  import subprocess
  result = subprocess.run(['curl', '-s', 'https://example.com'], capture_output=True, text=True)
  print(result.stdout)
  ```
- **HTTP requests without installing packages**: Use `urllib.request`:
  ```python
  import urllib.request, json
  req = urllib.request.Request(url, data=json.dumps(payload).encode(), headers=headers, method='POST')
  with urllib.request.urlopen(req) as resp:
      print(json.loads(resp.read()))
  ```

## Important: No Host Env Vars

Environment variables from the distri server are NOT available in the shell.
Do NOT use `os.getenv('GOOGLE_TOKEN')` — it will be None.

If you need an OAuth token for API calls, use `connection_request` via `distri_platform` instead of code execution. The token is only available server-side.

## When to Use Code vs connection_request

| Need | Use |
|------|-----|
| Call an API with OAuth token | `connection_request` (token auto-injected) |
| Process/transform data | `call_code` with Python |
| Complex multi-step computation | `call_code` with Python |
| Generate files/charts | `call_code` with Python |
| Install and use libraries | `call_code` (pip install via subprocess) |

## Example: HTTP Request with urllib (no install needed)

```python
import urllib.request, json

url = "https://api.example.com/data"
headers = {"Authorization": "Bearer TOKEN", "Content-Type": "application/json"}
data = json.dumps({"key": "value"}).encode()

req = urllib.request.Request(url, data=data, headers=headers, method="POST")
with urllib.request.urlopen(req) as resp:
    result = json.loads(resp.read())
    print(json.dumps(result, indent=2))
```

## Example: Install and use a package

```python
import subprocess, sys
subprocess.check_call([sys.executable, '-m', 'pip', 'install', '-q', 'requests'])

import requests
resp = requests.get("https://api.example.com/data")
print(resp.json())
```
