# Distri Coder Sample

This sample demonstrates the **Distri Coder** agent ("codela"), designed for iterative software development tasks.

## Prerequisites

- `distri` CLI installed.
- Access to a running Distri server.

## Setup

1. **Start the Distri Server** (if not running):
   ```bash
   distri serve
   ```

2. **Push the Agent Definition**:
   Push the coder agent definition to the server:
   ```bash
   distri agents push agents/coder.md
   ```

## Usage

Use the `distri run` command to invoke the coder agent. You can specify the task you want it to perform.

**Example: Create a Hello World**
```bash
distri run --agent codela --task "Create a simple Python script that prints 'Hello World' and save it to hello.py"
```

**Note on Workspace**:
The agent operates within the server's working directory or the specific workspace configured for the session. Ensure your server or client has access to the target files.
