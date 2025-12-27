# @distri/home

Reusable dashboard components for Distri Framework.

## Installation

```bash
pnpm add @distri/home
```

## Usage

```tsx
import { DistriProvider } from '@distri/react';
import { DistriHomeProvider, Home, AgentDetails, ThreadsView, SettingsView } from '@distri/home';
import { useNavigate } from 'react-router-dom';

function App() {
  const navigate = useNavigate();

  return (
    <DistriProvider config={{ baseUrl: 'YOUR_API_URL' }}>
      <DistriHomeProvider
        onNavigate={navigate}
        config={{
          enableApiKeys: false, // Set to true for cloud version
        }}
      >
        {/* Use components in your routes */}
        <Home />
      </DistriHomeProvider>
    </DistriProvider>
  );
}
```

## Components

- **Home** - Dashboard overview with stats and recent threads
- **AgentDetails** - Agent details with chat, tools, and integration tabs
- **ThreadsView** - Thread list with search and delete functionality
- **SettingsView** - Settings page with optional API keys management

## Configuration

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enableApiKeys` | boolean | true | Show API keys section in settings |

## Peer Dependencies

- `@distri/core`
- `@distri/react`
- `react` >= 18.0.0
- `react-dom` >= 18.0.0
