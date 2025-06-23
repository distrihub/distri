# LangDB Frontend

A modern AI chat interface built with Next.js 14, Vercel AI SDK, and Tailwind CSS. This application provides a sleek, responsive chat interface that connects to your Distri server and supports agent-to-agent (A2A) communication.

## Features

- 🤖 **AI-Powered Chat**: Built with Vercel AI SDK for seamless AI integration
- 🎨 **Modern UI/UX**: Clean, responsive design with smooth animations
- 🌙 **Dark Mode**: Full dark/light mode support with system preference detection
- 🔧 **Agent Integration**: Connect to specialized Distri agents through A2A protocol
- ⚡ **Real-time Streaming**: Live response streaming for better user experience
- 🎯 **TypeScript**: Full type safety throughout the application
- 🎨 **Tailwind CSS**: Utility-first CSS framework for rapid development

## Tech Stack

- **Framework**: Next.js 14 (App Router)
- **AI Integration**: Vercel AI SDK
- **Styling**: Tailwind CSS
- **Language**: TypeScript
- **Icons**: Lucide React
- **Theme**: next-themes
- **Backend Integration**: Distri Server (A2A Protocol)

## Prerequisites

- Node.js 18.0.0 or higher
- npm, yarn, or pnpm
- Running Distri server instance (optional for basic usage)

## Installation

1. **Clone the repository**:
   ```bash
   git clone <repository-url>
   cd langdb-frontend
   ```

2. **Install dependencies**:
   ```bash
   npm install
   # or
   yarn install
   # or
   pnpm install
   ```

3. **Set up environment variables**:
   Copy `.env.local.example` to `.env.local` and configure:
   
   ```env
   # OpenAI API Key for Langdb Gateway
   OPENAI_API_KEY=sk-your-openai-api-key-here
   LANGDB_API_KEY=your-langdb-api-key-here
   LANGDB_PROJECT_ID=your-langdb-project-id-here
   
   # Distri Server Configuration
   DISTRI_SERVER_URL=http://127.0.0.1:8080
   DISTRI_A2A_ENDPOINT=http://127.0.0.1:8080/api/v1
   
   # Next.js Configuration
   NEXTAUTH_SECRET=your-nextauth-secret-here
   NEXTAUTH_URL=http://localhost:3000
   ```

4. **Start the development server**:
   ```bash
   npm run dev
   # or
   yarn dev
   # or
   pnpm dev
   ```

5. **Open your browser** and navigate to [http://localhost:3000](http://localhost:3000)

## Configuration

### LangDB Integration

The application supports both direct OpenAI API calls and LangDB gateway routing:

- **Direct OpenAI**: Set only `OPENAI_API_KEY`
- **LangDB Gateway**: Set `LANGDB_API_KEY` and optionally `LANGDB_PROJECT_ID`

### Distri Server Integration

To enable agent mode and connect to specialized Distri agents:

1. Ensure your Distri server is running
2. Configure `DISTRI_SERVER_URL` and `DISTRI_A2A_ENDPOINT`  
3. Toggle "Agent Mode" in the application interface

## Usage

### Basic Chat

1. Open the application
2. Start typing in the input field
3. Press Enter or click the send button
4. View AI responses in real-time

### Agent Mode

1. Toggle "Agent Mode" in the top-right corner
2. Select an available agent from the dropdown
3. Your messages will be routed through the selected Distri agent
4. Responses will include agent-specific capabilities

### Dark Mode

- Use the theme toggle in the top-right corner
- Choose between Light, Dark, or System theme
- Theme preference is automatically saved

## API Endpoints

### POST /api/chat

Main chat endpoint that handles:
- Direct OpenAI API calls
- LangDB gateway routing
- Distri agent communication
- Real-time response streaming

**Request Body**:
```json
{
  "messages": [...],         // Chat history
  "useDistriAgent": false,   // Enable agent routing
  "agentId": "agent-id"      // Selected agent ID
}
```

### GET /api/chat

Retrieves available Distri agents:

**Response**:
```json
{
  "agents": [
    {
      "id": "agent-1",
      "name": "Agent Name",
      "description": "Agent description",
      "capabilities": {...}
    }
  ]
}
```

## Project Structure

```
langdb-frontend/
├── src/
│   ├── app/
│   │   ├── api/chat/          # Chat API routes
│   │   ├── globals.css        # Global styles
│   │   ├── layout.tsx         # Root layout
│   │   └── page.tsx           # Home page
│   ├── components/
│   │   ├── chat-interface.tsx # Main chat component
│   │   ├── theme-provider.tsx # Theme context
│   │   └── theme-toggle.tsx   # Theme switcher
│   └── lib/                   # Utility functions
├── public/                    # Static assets
├── .env.local                 # Environment variables
├── tailwind.config.ts         # Tailwind configuration
└── package.json              # Dependencies and scripts
```

## Customization

### Styling

The application uses a custom design system built on Tailwind CSS. You can customize:

- **Colors**: Edit CSS variables in `globals.css`
- **Components**: Modify component styles in their respective files
- **Animations**: Add custom animations in `tailwind.config.ts`

### Chat Behavior

Customize chat behavior by modifying:

- **System prompts**: Edit the system message in `/api/chat/route.ts`
- **Model parameters**: Adjust temperature, max tokens, etc.
- **Agent integration**: Customize A2A protocol handling

## Deployment

### Vercel (Recommended)

1. **Push to GitHub/GitLab**
2. **Connect to Vercel**
3. **Configure environment variables**
4. **Deploy**

### Other Platforms

The application can be deployed to any platform supporting Next.js:
- Netlify
- Railway
- DigitalOcean App Platform
- Self-hosted with Docker

## Development

### Available Scripts

- `npm run dev` - Start development server
- `npm run build` - Build for production
- `npm run start` - Start production server
- `npm run lint` - Run ESLint

### Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Test thoroughly
5. Submit a pull request

## Troubleshooting

### Common Issues

**API Key Issues**:
- Verify your OpenAI/LangDB API key is correct
- Check that the key has sufficient credits/permissions

**Distri Server Connection**:
- Ensure Distri server is running on the specified port
- Check network connectivity and CORS settings
- Verify A2A endpoint configuration

**Theme Issues**:
- Clear browser cache and localStorage
- Check if JavaScript is enabled
- Verify next-themes installation

**Build Errors**:
- Delete `.next` folder and `node_modules`
- Run `npm install` again
- Check for TypeScript errors

## Support

For support and questions:
- Check the [GitHub Issues](issues-url)
- Review the [Distri Documentation](distri-docs-url)
- Consult [Vercel AI SDK Documentation](https://sdk.vercel.ai)

## License

This project is licensed under the MIT License - see the LICENSE file for details.

## Acknowledgments

- [Vercel AI SDK](https://sdk.vercel.ai) for AI integration
- [Tailwind CSS](https://tailwindcss.com) for styling
- [Lucide React](https://lucide.dev) for icons
- [Next.js](https://nextjs.org) for the framework
