# Distri: A Composable Agent Framework
Distri is a framework for building and composing AI agents, written in Rust. It enables developers to create, publish, and combine agent capabilities using the MCP (Multi-Agent Communication Protocol) standard.

<p align="center">
  <img src="https://raw.githubusercontent.com/distrihub/distri/refs/heads/main/images/help.png" alt="Distri Screenshot" width="600"/>
</p>

## Getting Started
Distri agents can be configured and run in two ways:

### 1. YAML Configuration

### 2. Rust Scripts (Advanced Workflows)  **Coming Soon**

Lets explore running `distri` using a [sample configuration file](https://raw.githubusercontent.com/distrihub/distri/samples/config.yaml). 

List Agents
```bash
distri list -c samples/config.yaml
```
<p align="center">
  <img src="https://raw.githubusercontent.com/distrihub/distri/refs/heads/main/images/agents.png" alt="Distri Agents" width="600"/>
</p>

You can run `github_explorer` using:
```bash
distri run -c samples/config.yaml github_explorer
```



## Installation

You can install Distri in two ways:

### Using Cargo

```bash
cargo install --git https://github.com/distrihub/distri distri --locked
```

### Using Docker

```bash
docker run -it distrihub/distri
```

## MCP Proxy & Tools

Distri proxy also provides a convenient proxy to run stdio commands.
```bash
distri proxy -c samples/proxy.yaml
```

For looking at all the available tools
```bash
distri list-tools -c samples/config.yaml
```
<p align="center">
  <img src="https://raw.githubusercontent.com/distrihub/distri/refs/heads/main/images/tools.png" alt="MCPs available via proxy" width="600"/>
</p>


## AI Gateway
Distri is connected to AI Gateway and has access to 250+ LLMs. For more details checkout [Langdb AI Gateway](https://langdb.ai/).

## What is MCP?

MCP (Multi-Agent Communication Protocol) is a standardized protocol that enables agents to:
- Communicate with each other in a structured way
- Share capabilities and tools
- Execute tasks collaboratively
- Handle state management and coordination

With MCP, any agent can be published as a reusable tool that other agents can leverage, creating an ecosystem of composable AI capabilities.

## Features

- 🔧 **Modular Design**: Build agents as independent modules that can be mixed and matched
- 🤝 **MCP Protocol**: Standardized communication between agents
- 🚀 **Rust Performance**: Built with Rust for reliability and speed
- 📦 **Tool Publishing**: Share your agents as MCP-compatible tools
- 🔌 **Easy Integration**: Simple API for adding new capabilities

## Configuration

Distri agents can be configured in two ways:

### 1. YAML Configuration

### 2. Rust Scripts (Advanced Workflows)  **Coming Soon**

## Status

⚠️ **Early Development**: Distri is in early stages of development. APIs and protocols may change as we gather feedback and improve the framework.

## Getting Started

[Documentation and examples coming soon]

## License

This project is licensed under the Apache License 2.0 - see the [LICENSE](LICENSE) file for details.

## Contributing

We welcome contributions! Please see our [CONTRIBUTING.md](CONTRIBUTING.md) guide for details. 