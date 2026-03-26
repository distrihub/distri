# Distri Coder Sample

This sample demonstrates the **codela** agent, a CLI-based coding assistant designed for iterative software development tasks.

> **Note**: This sample showcases the distri CLI capabilities and requires a complete distri CLI installation to run.

## Quick Start with Distri Cloud

1. **Install Distri CLI**:
   ```bash
   curl -fsSL https://distri.dev/install.sh | bash
   ```

2. **Push the Agent to Distri Cloud**:
   ```bash
   distri push
   ```

3. **Run Tasks**:
   ```bash
   distri run --agent codela --task "Create a simple Python script that prints 'Hello World'"
   ```

## Local Development (Optional)

If you prefer to run a local distri server:

1. **Start the Local Server**:
   ```bash
   distri serve
   ```

2. **Push the Agent Locally**:
   ```bash
   distri push --local
   ```

3. **Run Tasks**:
   ```bash
   distri run --agent codela --task "Your task here"
   ```

## Features

- Iterative code generation and refinement
- File system operations (read/write/edit files)
- Shell command execution
- Code review and analysis

## Example Tasks

```bash
# Create a new file
distri run --agent codela --task "Create a hello.py script that prints Hello World"

# Edit existing code
distri run --agent codela --task "Add error handling to the main function in app.py"

# Code review
distri run --agent codela --task "Review the code in src/ and suggest improvements"
```

## Configuration

The agent configuration is defined in `agents/coder.md`. Customize the system prompt and capabilities as needed.

## Workspace

The agent operates within the current working directory. Set the `CODE_HOME` environment variable to specify a different workspace root.
