import { useEffect, useState } from 'react';
import { DistriProvider, ThemeProvider } from '@distri/react';
import { Chat } from '@distri/react';
import { TokenProvider, useInitialization } from '@/components/TokenProvider';
import { ThreadProvider } from '@/components/ThreadContext';
import { BACKEND_URL } from './constants';
import { AccountProvider } from './components/AccountProvider';

function TauriApp() {
  // Initialize theme to dark by default
  useEffect(() => {
    const currentTheme = localStorage.getItem('distri-theme');
    if (!currentTheme || currentTheme === 'system') {
      localStorage.setItem('distri-theme', 'dark');
    }
  }, []);

  return (
    <ThemeProvider defaultTheme="dark" storageKey="distri-theme">
      <ThreadProvider>
        <TokenProvider>
          <AccountProvider>
            <WrappedTauriContent />
          </AccountProvider>
        </TokenProvider>
      </ThreadProvider>
    </ThemeProvider>
  );
}

const WrappedTauriContent = () => {
  const { token, setToken } = useInitialization();
  const [threadId] = useState(() => `tauri-${Date.now()}`);

  // For Tauri app, we'll use a simple token approach or local development
  useEffect(() => {
    // Set a development token or allow anonymous access
    const storedToken = localStorage.getItem('distri_desktop_token');
    if (storedToken) {
      setToken(storedToken);
    } else {
      // For development, set a placeholder token
      setToken('desktop-dev-token');
    }
  }, [setToken]);

  return (
    <DistriProvider config={{
      baseUrl: `${BACKEND_URL}/api/v1/`,
      headers: token ? {
        'Authorization': `Bearer ${token}`
      } : {},
      interceptor: async (init?: RequestInit): Promise<RequestInit | undefined> => {
        const initCopy = init || {}
        const newInit = {
          ...initCopy,
          headers: {
            ...initCopy.headers,
            ...(token ? { 'Authorization': `Bearer ${token}` } : {})
          }
        }
        return newInit
      }
    }}>
      <div className="h-screen w-full bg-background text-foreground">
        {/* Header */}
        <div className="border-b border-border bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
          <div className="flex h-14 items-center px-4">
            <h1 className="text-lg font-semibold">Distri Desktop</h1>
            <div className="ml-auto text-sm text-muted-foreground">
              Connected to distri-server
            </div>
          </div>
        </div>
        
        {/* Chat Interface */}
        <div className="h-[calc(100vh-3.5rem)]">
          <Chat
            threadId={threadId}
            theme="dark"
            voiceEnabled={true}
            ttsConfig={{
              model: 'openai',
              voice: 'alloy',
              speed: 1.0
            }}
          />
        </div>
      </div>
    </DistriProvider>
  );
}

export default TauriApp;
