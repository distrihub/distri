# LangDB Frontend Demo

This demo showcases the full-featured AI chat interface built with Vercel AI SDK, connecting to Distri server and supporting LangDB gateway integration.

## 🚀 Quick Start

1. **Install dependencies**:
   ```bash
   npm install
   ```

2. **Configure environment** (copy and edit `.env.local`):
   ```env
   OPENAI_API_KEY=sk-your-openai-api-key-here
   LANGDB_API_KEY=your-langdb-api-key-here
   LANGDB_PROJECT_ID=your-langdb-project-id-here
   DISTRI_SERVER_URL=http://127.0.0.1:8080
   DISTRI_A2A_ENDPOINT=http://127.0.0.1:8080/api/v1
   ```

3. **Start the development server**:
   ```bash
   npm run dev
   ```

4. **Open http://localhost:3000**

## 🎯 Features Demonstration

### 1. Basic AI Chat
- **Modern Interface**: Clean, responsive design with smooth animations
- **Real-time Streaming**: Messages stream in real-time using Vercel AI SDK
- **Markdown Support**: Full markdown rendering for rich AI responses
- **Dark/Light Mode**: Seamless theme switching with system preference support

### 2. Agent Integration (A2A Protocol)
- **Toggle Agent Mode**: Enable/disable specialized Distri agent routing
- **Agent Selection**: Dropdown to choose from available agents
- **Fallback Handling**: Graceful fallback to direct OpenAI if agent fails
- **Dynamic Context**: Messages adapt based on selected agent capabilities

### 3. LangDB Gateway Support
- **Dual Configuration**: Supports both direct OpenAI and LangDB gateway
- **Project Isolation**: LangDB project ID for request routing
- **Header Management**: Automatic X-Project-Id header injection
- **Cost Optimization**: Token usage optimization through LangDB

## 🎨 UI/UX Highlights

### Design System
- **Blink-UI Style**: Modern, minimalist design inspired by blink-ui
- **Glassmorphism**: Subtle backdrop blur effects
- **Smooth Animations**: CSS animations for message bubbles and loading states
- **Responsive Layout**: Mobile-first design that works on all devices

### Dark Mode Implementation
- **System Detection**: Automatically detects user's system preference
- **Manual Toggle**: Three-state toggle (Light/Dark/System)
- **Smooth Transitions**: CSS transitions for theme changes
- **Persistent Storage**: Theme preference saved across sessions

### Interactive Elements
- **Floating Send Button**: Modern circular send button with loading states
- **Auto-resize Textarea**: Input field grows with content
- **Keyboard Shortcuts**: Enter to send, Shift+Enter for new line
- **Loading Indicators**: Spinning icons and typing indicators

## 🔧 Technical Architecture

### Frontend Stack
- **Next.js 14**: App Router with React Server Components
- **Vercel AI SDK**: `useChat` hook for seamless AI integration
- **Tailwind CSS**: Utility-first styling with custom design system
- **TypeScript**: Full type safety throughout the application
- **Lucide React**: Consistent icon library

### Backend Integration
- **API Routes**: Next.js API routes handling chat requests
- **Streaming Support**: Real-time response streaming
- **Error Handling**: Comprehensive error handling and fallbacks
- **A2A Protocol**: Full support for Distri agent communication

### State Management
- **React Hooks**: useState and useEffect for local state
- **Theme Context**: next-themes for theme management
- **Chat State**: Vercel AI SDK managing conversation state
- **Agent State**: Custom state for agent selection and configuration

## 🌐 API Endpoints

### POST /api/chat
**Purpose**: Main chat endpoint for AI interactions

**Request Body**:
```json
{
  "messages": [
    {
      "role": "user", 
      "content": "Hello, how are you?"
    }
  ],
  "useDistriAgent": false,
  "agentId": "optional-agent-id"
}
```

**Response**: Streaming text response via Vercel AI SDK

**Features**:
- Real-time streaming
- LangDB gateway support
- Distri agent routing
- Error handling and fallbacks

### GET /api/chat
**Purpose**: Retrieve available Distri agents

**Response**:
```json
{
  "agents": [
    {
      "id": "agent-1",
      "name": "Code Assistant",
      "description": "Specialized in code generation and debugging",
      "capabilities": {...}
    }
  ]
}
```

## 🎮 Demo Scenarios

### Scenario 1: Basic Conversation
1. Open the application
2. Type "Hello, can you help me with programming?"
3. Watch the real-time streaming response
4. Try follow-up questions to see conversation context

### Scenario 2: Agent Mode
1. Toggle "Agent Mode" in the top-right
2. Select an agent from the dropdown (if available)
3. Ask specialized questions relevant to the agent
4. Compare responses with and without agent mode

### Scenario 3: Dark Mode
1. Use the theme toggle in the top-right corner
2. Switch between Light, Dark, and System modes
3. Notice smooth transitions and consistent styling
4. Refresh page to see persistent theme preference

### Scenario 4: Mobile Experience
1. Open on mobile device or resize browser window
2. Test responsive layout and touch interactions
3. Verify keyboard behavior and scrolling
4. Check theme toggle and agent selection on mobile

## 🔧 Customization Points

### Styling Customization
```css
/* Edit src/app/globals.css */
:root {
  --primary: 222.2 47.4% 11.2%;        /* Primary color */
  --background: 0 0% 100%;              /* Background color */
  --foreground: 222.2 84% 4.9%;        /* Text color */
}
```

### Chat Behavior
```typescript
// Edit src/app/api/chat/route.ts
const result = await streamText({
  model: openaiClient('gpt-4o-mini'),
  temperature: 0.7,                     /* Creativity level */
  maxTokens: 1000,                      /* Response length */
  // ... other parameters
});
```

### UI Components
```typescript
// Edit src/components/chat-interface.tsx
// Customize message bubbles, animations, layout
```

## 🚀 Deployment

### Vercel Deployment
1. Push code to GitHub/GitLab
2. Connect repository to Vercel
3. Configure environment variables in Vercel dashboard
4. Deploy with automatic builds on push

### Environment Variables (Production)
```env
OPENAI_API_KEY=sk-prod-key
LANGDB_API_KEY=prod-langdb-key
LANGDB_PROJECT_ID=prod-project-id
DISTRI_SERVER_URL=https://your-distri-server.com
DISTRI_A2A_ENDPOINT=https://your-distri-server.com/api/v1
```

## 🧪 Testing Guide

### Manual Testing Checklist
- [ ] Basic chat functionality
- [ ] Real-time message streaming
- [ ] Theme switching (Light/Dark/System)
- [ ] Agent mode toggle and selection
- [ ] Mobile responsiveness
- [ ] Error handling (invalid API key, network issues)
- [ ] Message history and context
- [ ] Keyboard shortcuts (Enter, Shift+Enter)
- [ ] Loading states and animations

### Performance Testing
- [ ] Initial page load time
- [ ] Time to first message response
- [ ] Theme switching performance
- [ ] Scroll performance with many messages
- [ ] Memory usage during long conversations

## 📱 Browser Support

### Tested Browsers
- ✅ Chrome 90+
- ✅ Firefox 88+
- ✅ Safari 14+
- ✅ Edge 90+
- ✅ Mobile browsers (iOS Safari, Chrome Mobile)

### Features
- ✅ CSS Grid and Flexbox
- ✅ CSS Custom Properties
- ✅ Fetch API
- ✅ ES6+ JavaScript
- ✅ WebSocket (for streaming)

## 🎯 Future Enhancements

### Planned Features
- [ ] Message export functionality
- [ ] Conversation history persistence
- [ ] File upload support
- [ ] Voice input/output
- [ ] Multiple concurrent agent conversations
- [ ] Advanced agent configuration
- [ ] Custom prompt templates
- [ ] Analytics and usage tracking

### Technical Improvements
- [ ] Service Worker for offline support
- [ ] Message caching strategies
- [ ] Performance optimizations
- [ ] A11y improvements
- [ ] Internationalization (i18n)
- [ ] Advanced error recovery

---

## 💡 Tips & Tricks

1. **Use Keyboard Shortcuts**: Press Enter to send, Shift+Enter for new lines
2. **Theme Follows System**: Set to "System" to automatically match device preference
3. **Agent Specialization**: Different agents excel at different tasks
4. **Long Conversations**: Scroll performance is optimized for extended chats
5. **Mobile First**: Designed primarily for mobile, desktop is enhanced experience

---

Built with ❤️ using Vercel AI SDK, Next.js, and Tailwind CSS