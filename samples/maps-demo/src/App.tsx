import { useMemo, useState, useCallback } from 'react';
import { DistriProvider, Chat, useAgent, DistriAnyTool, useThreads, useChatMessages } from '@distri/react';
import { AlertCircle } from 'lucide-react';
import { APIProvider } from '@vis.gl/react-google-maps';
import type { GoogleMapsManagerRef } from './components/GoogleMapsManager';
import Layout from './components/Layout';
import { getTools } from './Tools.tsx';
import { Agent, uuidv4 } from '@distri/core';

// Environment variables validation
const GOOGLE_MAPS_API_KEY = import.meta.env.VITE_GOOGLE_MAPS_API_KEY;
const DISTRI_API_URL = import.meta.env.VITE_DISTRI_API_URL || 'http://localhost:8080/api/v1';

function getThreadId() {
  const threadId = localStorage.getItem('MapsDemo:threadId');
  if (!threadId) {
    const newThreadId = uuidv4();
    localStorage.setItem('MapsDemo:threadId', newThreadId);
    return newThreadId;
  }
  return threadId;
}

function MapsContent() {
  const { agent, loading } = useAgent({ agentIdOrDef: 'maps_agent' });
  const [selectedThreadId, setSelectedThreadId] = useState<string>(getThreadId());
  const [tools, setTools] = useState<DistriAnyTool[]>([]);
  const [voiceEnabled, setVoiceEnabled] = useState<boolean>(false);

  const { messages } = useChatMessages({
    agent: agent as Agent,
    threadId: selectedThreadId,
    onError: (error) => {
      console.error('Error fetching messages:', error);
    }
  });
  // Thread management
  const { threads, loading: threadsLoading, refetch, deleteThread } = useThreads();

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
        const newThreadId = uuidv4();
        setSelectedThreadId(newThreadId);
        localStorage.setItem('MapsDemo:threadId', newThreadId);
      }
    } catch (error) {
      console.error('Failed to delete thread:', error);
    }
  }, [deleteThread, selectedThreadId]);

  // New chat logic
  const handleNewChat = useCallback(() => {
    const newThreadId = uuidv4();
    setSelectedThreadId(newThreadId);
    localStorage.setItem('MapsDemo:threadId', newThreadId);
    // Optionally, refetch threads to show the new one if backend supports creation
  }, []);

  if (loading) {
    return (
      <div className="flex h-screen items-center justify-center bg-gray-100 dark:bg-gray-950">
        <div className="flex items-center space-x-2">
          <div className="h-6 w-6 animate-spin rounded-full border-2 border-blue-600 border-t-transparent"></div>
          <span className="text-gray-600 dark:text-gray-300">Loading Maps Assistant...</span>
        </div>
      </div>
    );
  }

  if (!agent) {
    return (
      <div className="flex h-screen items-center justify-center bg-gray-100 dark:bg-gray-950">
        <div className="max-w-md p-6 bg-white dark:bg-gray-900 rounded-lg shadow-lg text-center">
          <AlertCircle className="h-12 w-12 text-red-500 mx-auto mb-4" />
          <h2 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-2">
            Backend Not Available
          </h2>
          <p className="text-gray-600 dark:text-gray-300 mb-4">
            The Distri backend server is not running. Please start the backend server.
          </p>
          <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-4 mb-4">
            <p className="text-sm text-gray-700 dark:text-gray-300 mb-2">
              To start the backend, run:
            </p>
            <code className="bg-gray-200 dark:bg-gray-700 px-2 py-1 rounded text-sm">
              pnpm dev:maps
            </code>
          </div>
          <p className="text-xs text-gray-500 dark:text-gray-400">
            Make sure the backend is running on {DISTRI_API_URL}
          </p>
        </div>
      </div>
    );
  }

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
      {agent && tools.length > 0 ? (
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

          <div className="w-96 overflow-hidden">
            <Chat
              agent={agent}
              externalTools={tools}
              initialMessages={messages}
              theme="dark"
              threadId={selectedThreadId}
              voiceEnabled={voiceEnabled}
              maxWidth="384px"
              ttsConfig={{
                model: 'openai',
                voice: 'alloy',
                speed: 1.0
              }}
            />
          </div>
        </div>
      ) : (
        <div className="flex items-center justify-center h-full bg-gray-900 text-gray-300">
          <div className="text-center">
            <div className="h-8 w-8 animate-spin rounded-full border-2 border-blue-600 border-t-transparent mx-auto mb-2"></div>
            <p>Loading chat interface...</p>
            {!agent && <p className="text-sm text-gray-400 mt-1">Waiting for agent</p>}
            {agent && tools.length === 0 && <p className="text-sm text-gray-400 mt-1">Waiting for map tools</p>}
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
    debug: true
  }), []);

  // Check for required environment variables
  const envCheck = EnvironmentCheck();
  if (envCheck) return envCheck;

  return (
    <DistriProvider config={config}>
      <APIProvider apiKey={GOOGLE_MAPS_API_KEY} libraries={["places", "routes"]}>
        <MapsContent />
      </APIProvider>
    </DistriProvider>
  );
}

export default App;