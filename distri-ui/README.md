# Distri Frontend

A React frontend application for the Distri AI Agent Platform using AG-UI protocol for agent communication.

## Features

- **Agent Chat Interface**: Real-time chat with AI agents using A2A protocol
- **Agent Management**: View and manage available agents
- **Task Monitoring**: Track task status and execution in real-time
- **AG-UI Integration**: Built with AG-UI for seamless agent interaction
- **Modern UI**: Beautiful, responsive interface built with Tailwind CSS

## Prerequisites

- Node.js 18+ and npm/yarn
- Distri server running on port 8080

## Installation

1. Install dependencies:
```bash
npm install
```

2. Start the development server:
```bash
npm run dev
```

3. Open http://localhost:3000 in your browser

## Usage

### Starting a Chat
1. The frontend will automatically load available agents from the Distri server
2. Select an agent from the sidebar
3. Start typing in the chat interface
4. Messages are sent using the A2A protocol to the agent and responses are displayed in real-time

### Monitoring Tasks
1. Click on the "Tasks" tab to view task execution status
2. Tasks are automatically updated in real-time
3. You can see task state changes and history

### Managing Agents
1. Click on the "Agents" tab to view all available agents
2. See agent status, descriptions, and capabilities
3. Refresh the agent list to get the latest information

## Architecture

The frontend communicates with the Distri server using:
- **A2A Protocol**: For sending messages and managing tasks
- **JSON-RPC**: Standard protocol for API communication
- **Server-Sent Events (SSE)**: For real-time updates and streaming
- **AG-UI**: Protocol-compliant agent interaction framework

## Configuration

The frontend is configured to proxy API requests to `http://localhost:8080`. This can be changed in `vite.config.ts`.

## Development

```bash
# Start development server
npm run dev

# Build for production
npm run build

# Preview production build
npm run preview

# Lint code
npm run lint
```

## Dependencies

- **React 18**: Modern React with hooks
- **AG-UI**: Agent-User Interaction Protocol implementation
- **Tailwind CSS**: Utility-first CSS framework
- **Lucide React**: Beautiful icon library
- **Vite**: Fast build tool and dev server