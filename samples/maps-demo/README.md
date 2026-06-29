# Maps Demo

A Google Maps integration sample for the Distri Framework using react-google-maps.

> **Live Demo**: This sample is deployed to GitHub Pages and can be embedded as an iframe.

## Quick Start with Distri Cloud

1. **Install Distri CLI**:
   ```bash
   curl -fsSL https://distri.dev/install.sh | bash
   ```

2. **Push to Distri Cloud**:
   ```bash
   distri push
   ```

3. **Access your app** through the Distri dashboard or via API.

## Local Development

1. **Install Dependencies**:
   ```bash
   pnpm install
   ```

2. **Configure Environment**:
   ```bash
   cp .env.example .env
   # Edit .env and set your VITE_GOOGLE_MAPS_API_KEY
   ```

3. **Start Development Server**:
   ```bash
   pnpm dev
   ```

4. **Optional: Connect to Local Distri Server**:
   ```bash
   # In .env, set:
   VITE_DISTRI_API_URL=http://localhost:8080/v1
   ```

## Features

- **Interactive Maps**: Full Google Maps integration with custom tools
- **AI Chat Interface**: Chat with an AI assistant that can interact with the map
- **🎤 Voice Support**: Toggle voice recording for hands-free map navigation
- **Thread Management**: Create, rename, and delete conversation threads

## Iframe Embedding

Once deployed, embed this sample in your application:

```html
<iframe 
  src="https://distrihub.github.io/distri/samples/maps" 
  width="100%" 
  height="600"
  frameborder="0">
</iframe>
```

## Environment Variables

- `VITE_GOOGLE_MAPS_API_KEY`: Your Google Maps API key (required)
- `VITE_DISTRI_API_URL`: Distri API server URL (defaults to cloud)

## Dependencies

- `@distri/react`: Distri React components
- `@distri/core`: Distri core functionality
- `@vis.gl/react-google-maps`: Google Maps React components
- `lucide-react`: Icons