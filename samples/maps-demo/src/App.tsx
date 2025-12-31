import { useMemo, useState, useCallback } from 'react';
import { DistriProvider, Chat, ChatEmbed, DistriAnyTool, useThreads } from '@distri/react';
import { AlertCircle } from 'lucide-react';
import { APIProvider } from '@vis.gl/react-google-maps';
import type { GoogleMapsManagerRef } from './components/GoogleMapsManager';
import Layout from './components/Layout';
import { getTools } from './Tools.tsx';

// Environment variables validation
const GOOGLE_MAPS_API_KEY = import.meta.env.VITE_GOOGLE_MAPS_API_KEY;
const DISTRI_API_URL = import.meta.env.VITE_DISTRI_API_URL || 'http://localhost:8080/v1';
const CLIENT_ID = import.meta.env.VITE_DISTRI_CLIENT_ID || 'dpc_FRpvDH5ZzLrNF9vWto57Nfl8bd6wKtcD';
const EMBED_URL = import.meta.env.VITE_DISTRI_EMBED_URL;
// When true, use ChatEmbed with iframe + Turnstile auth (for deployed demos)
const USE_EMBED = import.meta.env.VITE_USE_EMBED === 'true';
console.log("USE_EMBED", USE_EMBED)

function getThreadId() {
  const threadId = localStorage.getItem('MapsDemo:threadId');
  if (!threadId) {
    const newThreadId = crypto.randomUUID();
    localStorage.setItem('MapsDemo:threadId', newThreadId);
    return newThreadId;
  }
  return threadId;
}

function MapsContent() {
  const [selectedThreadId, setSelectedThreadId] = useState<string>(getThreadId());
  const [tools, setTools] = useState<DistriAnyTool[]>([]);
  const [voiceEnabled, setVoiceEnabled] = useState<boolean>(false);


  // Thread management
  const { threads, loading: threadsLoading, refetch, deleteThread } = useThreads({ enabled: !USE_EMBED });

  // Get tools when map manager is ready
  const handleMapReady = useCallback((mapRef: GoogleMapsManagerRef) => {
    console.log('Map manager is ready, getting tools...');
    const mapTools = getTools(mapRef);
    console.log('Created tools array:', mapTools.map(t => t.name));
    setTools(mapTools);
  }, []);

  // Thread management functions
  const handleThreadSelect = useCallback((threadId: string) => {
    console.log('handleThreadSelect', threadId);
    setSelectedThreadId(threadId);
    localStorage.setItem('MapsDemo:threadId', threadId);
  }, []);

  const handleThreadDelete = useCallback(async (threadId: string) => {
    try {
      await deleteThread(threadId);
      if (selectedThreadId === threadId) {
        const newThreadId = crypto.randomUUID();
        setSelectedThreadId(newThreadId);
        localStorage.setItem('MapsDemo:threadId', newThreadId);
      }
    } catch (error) {
      console.error('Failed to delete thread:', error);
    }
  }, [deleteThread, selectedThreadId]);

  // New chat logic
  const handleNewChat = useCallback(() => {
    const newThreadId = crypto.randomUUID();
    setSelectedThreadId(newThreadId);
    localStorage.setItem('MapsDemo:threadId', newThreadId);
    // Optionally, refetch threads to show the new one if backend supports creation
  }, []);

  return (
    <Layout
      onMapReady={handleMapReady}
      threads={threads as any}
      selectedThreadId={selectedThreadId}
      loading={threadsLoading}
      onThreadSelect={handleThreadSelect}
      onThreadDelete={handleThreadDelete}
      onRefresh={refetch}
      onNewChat={handleNewChat}
    >
      {tools.length > 0 && (
        <div className="h-full flex flex-col">
          <div className="p-2 border-b border-gray-700 bg-gray-800">
            <button
              onClick={() => setVoiceEnabled(!voiceEnabled)}
              className={`px-3 py-1 rounded text-sm transition-colors ${voiceEnabled
                ? 'bg-green-600 hover:bg-green-700 text-white'
                : 'bg-gray-600 hover:bg-gray-700 text-gray-300'
                }`}
            >
              🎤 Voice {voiceEnabled ? 'ON' : 'OFF'}
            </button>
          </div>

          <div className="flex-1 overflow-hidden">
            {USE_EMBED ? (
              <ChatEmbed
                clientId={CLIENT_ID}
                agentId="maps_agent"
                theme="dark"
                baseUrl={DISTRI_API_URL}
                threadId={selectedThreadId}
                tools={tools}
                height="100%"
                enableHistory={true}
                embedUrl={EMBED_URL}
              />
            ) : (
              <Chat
                agentId="maps_agent"
                threadId={selectedThreadId}
                externalTools={tools}
                enableHistory={true}
                theme="dark"
                voiceEnabled={voiceEnabled}
                ttsConfig={{
                  model: 'openai',
                  voice: 'alloy',
                  speed: 1.0
                }}
              />
            )}
          </div>
        </div>
      )}
    </Layout>
  );
}

function EnvironmentCheck() {
  if (!GOOGLE_MAPS_API_KEY) {
    return (
      <div className="flex h-screen items-center justify-center bg-gray-50">
        <div className="max-w-md p-6 bg-white rounded-lg shadow-lg text-center">
          <AlertCircle className="h-12 w-12 text-red-500 mx-auto mb-4" />
          <h2 className="text-lg font-semibold text-gray-900 mb-2">
            Google Maps API Key Required
          </h2>
          <p className="text-gray-600 mb-4">
            To use this sample, you need to configure your Google Maps API key.
          </p>
          <div className="bg-gray-50 rounded-lg p-4 mb-4">
            <p className="text-sm text-gray-700 mb-2">
              1. Get an API key from the <a
                href="https://developers.google.com/maps/documentation/javascript/get-api-key"
                target="_blank"
                rel="noopener noreferrer"
                className="text-blue-600 hover:underline"
              >
                Google Maps Platform
              </a>
            </p>
            <p className="text-sm text-gray-700 mb-2">
              2. Copy <code className="bg-gray-200 px-1 rounded">.env.example</code> to <code className="bg-gray-200 px-1 rounded">.env</code>
            </p>
            <p className="text-sm text-gray-700">
              3. Set <code className="bg-gray-200 px-1 rounded">VITE_GOOGLE_MAPS_API_KEY</code> to your API key
            </p>
          </div>
          <p className="text-xs text-gray-500">
            Make sure to enable Maps JavaScript API and Places API in your Google Cloud Console
          </p>
        </div>
      </div>
    );
  }

  return null;
}

function App() {
  const config = useMemo(() => ({
    baseUrl: DISTRI_API_URL,
    clientId: CLIENT_ID,
    debug: true
  }), []);

  // Check for required environment variables
  const envCheck = EnvironmentCheck();
  if (envCheck) return envCheck;

  return (
    <DistriProvider config={config}>
      <APIProvider apiKey={GOOGLE_MAPS_API_KEY} libraries={["places"]}>
        <MapsContent />
      </APIProvider>
    </DistriProvider>
  );
}

export default App;