# Maps Demo

A Google Maps integration sample for the Distri Framework using react-google-maps.

## Features

- **Interactive Maps**: Full Google Maps integration with custom tools
- **AI Chat Interface**: Chat with an AI assistant that can interact with the map
- **🎤 Voice Support**: Toggle voice recording for hands-free map navigation
- **Header Navigation**: Clean header with app title and conversation toggle
- **Conversations Sidebar**: Right sidebar to view and manage recent conversations
- **Thread Management**: Create, rename, and delete conversation threads

## Layout

The app features a modern layout with:

- **Header**: Contains the app title "Maps Navigator" and a toggle button for the conversations sidebar
- **Main Content**: Split between Google Maps (left) and Chat interface (right)
- **Conversations Sidebar**: Right-side panel that slides in to show recent conversations

## Setup

1. Copy `.env.example` to `.env`
2. Set your Google Maps API key:
   ```
   VITE_GOOGLE_MAPS_API_KEY=your_api_key_here
   ```
3. Set your Distri API URL (optional):
   ```
   VITE_DISTRI_API_URL=http://localhost:8080/v1
   ```

## Development

```bash
pnpm install
pnpm dev
```

## Environment Variables

- `VITE_GOOGLE_MAPS_API_KEY`: Your Google Maps API key
- `VITE_DISTRI_API_URL`: Distri API server URL (defaults to localhost:8080)

Note: Voice support API keys are configured in the backend server, not the frontend.

## Dependencies

- `@distri/react`: Distri React components
- `@distri/core`: Distri core functionality
- `@distri/components`: Shared UI components
- `@vis.gl/react-google-maps`: Google Maps React components
- `lucide-react`: Icons