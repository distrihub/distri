import { useMemo, useState, useCallback } from 'react';
import { DistriProvider, Chat, DistriAnyTool, useThreads } from '@distri/react';
import { AlertCircle } from 'lucide-react';
import { APIProvider } from '@vis.gl/react-google-maps';
import type { GoogleMapsManagerRef } from './components/GoogleMapsManager';
import Layout from './components/Layout';
import { getTools } from './Tools.tsx';

// Environment variables validation
const GOOGLE_MAPS_API_KEY = import.meta.env.VITE_GOOGLE_MAPS_API_KEY;
const DISTRI_API_URL = (import.meta.env.VITE_DISTRI_API_URL || 'https://localhost:8081/v1').replace(/\/+$/, '');
const CLIENT_ID = import.meta.env.VITE_DISTRI_CLIENT_ID;

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
          <div className="flex-1 overflow-hidden">
            <Chat
              agentId="maps_agent"
              threadId={selectedThreadId}
              externalTools={tools}
              enableHistory={true}
              theme="dark"
            />
          </div>
        </div>
      )}
    </Layout>
  );
}

function EnvironmentCheck() {
  const [activeTab, setActiveTab] = useState<'cloud' | 'local'>('cloud');

  if (!GOOGLE_MAPS_API_KEY) {
    return (
      <div className="flex h-screen items-center justify-center bg-gray-50">
        <div className="max-w-xl w-full p-8 bg-white rounded-xl shadow-xl">
          <div className="text-center mb-8">
            <h2 className="text-2xl font-bold text-gray-900 mb-2">
              Setup Instructions
            </h2>
            <p className="text-gray-600">
              Follow these steps to get the maps-demo running.
            </p>
          </div>

          <div className="flex border-b border-gray-200 mb-6">
            <button
              onClick={() => setActiveTab('cloud')}
              className={`flex-1 py-3 text-sm font-medium border-b-2 transition-colors ${activeTab === 'cloud'
                ? 'border-blue-500 text-blue-600'
                : 'border-transparent text-gray-500 hover:text-gray-700 hover:border-gray-300'
                }`}
            >
              Distri Cloud
            </button>
            <button
              onClick={() => setActiveTab('local')}
              className={`flex-1 py-3 text-sm font-medium border-b-2 transition-colors ${activeTab === 'local'
                ? 'border-blue-500 text-blue-600'
                : 'border-transparent text-gray-500 hover:text-gray-700 hover:border-gray-300'
                }`}
            >
              Local Server
            </button>
          </div>

          <div className="space-y-6">
            {!GOOGLE_MAPS_API_KEY && (
              <div className="bg-red-50 border-l-4 border-red-400 p-4">
                <div className="flex">
                  <AlertCircle className="h-5 w-5 text-red-400 mr-3" />
                  <p className="text-sm text-red-700">
                    Google Maps API Key is missing. Get one from the{' '}
                    <a
                      href="https://developers.google.com/maps/documentation/javascript/get-api-key"
                      target="_blank"
                      rel="noopener noreferrer"
                      className="font-bold underline"
                    >
                      Google Cloud Console
                    </a>.
                  </p>
                </div>
              </div>
            )}

            {activeTab === 'cloud' ? (
              <div className="space-y-4">
                <div className="flex items-start">
                  <div className="flex-shrink-0 h-6 w-6 rounded-full bg-blue-100 text-blue-600 flex items-center justify-center text-xs font-bold mr-3 mt-0.5">1</div>
                  <p className="text-sm text-gray-700">Run <code className="bg-gray-100 px-1 rounded">distri push</code> to sync tools.</p>
                </div>
                <div className="flex items-start">
                  <div className="flex-shrink-0 h-6 w-6 rounded-full bg-blue-100 text-blue-600 flex items-center justify-center text-xs font-bold mr-3 mt-0.5">2</div>
                  <p className="text-sm text-gray-700">Get a <code className="bg-gray-100 px-1 rounded">clientId</code> from <a href="https://app.distri.dev" target="_blank" rel="noopener noreferrer" className="text-blue-600 hover:underline">app.distri.dev</a>.</p>
                </div>
                <div className="flex items-start">
                  <div className="flex-shrink-0 h-6 w-6 rounded-full bg-blue-100 text-blue-600 flex items-center justify-center text-xs font-bold mr-3 mt-0.5">3</div>
                  <p className="text-sm text-gray-700">Copy <code className="bg-gray-100 px-1 rounded">.env.example</code> to <code className="bg-gray-100 px-1 rounded">.env</code>.</p>
                </div>
                <div className="flex items-start">
                  <div className="flex-shrink-0 h-6 w-6 rounded-full bg-blue-100 text-blue-600 flex items-center justify-center text-xs font-bold mr-3 mt-0.5">4</div>
                  <p className="text-sm text-gray-700">Update <code className="bg-gray-100 px-1 rounded">VITE_DISTRI_CLIENT_ID</code> in your <code className="bg-gray-100 px-1 rounded">.env</code>.</p>
                </div>
              </div>
            ) : (
              <div className="space-y-4">
                <div className="flex items-start">
                  <div className="flex-shrink-0 h-6 w-6 rounded-full bg-blue-100 text-blue-600 flex items-center justify-center text-xs font-bold mr-3 mt-0.5">1</div>
                  <p className="text-sm text-gray-700">Run <code className="bg-gray-100 px-1 rounded">distri push</code> to sync tools.</p>
                </div>
                <div className="flex items-start">
                  <div className="flex-shrink-0 h-6 w-6 rounded-full bg-blue-100 text-blue-600 flex items-center justify-center text-xs font-bold mr-3 mt-0.5">2</div>
                  <p className="text-sm text-gray-700">Start your local server with <code className="bg-gray-100 px-1 rounded">distri serve</code>.</p>
                </div>
                <div className="flex items-start">
                  <div className="flex-shrink-0 h-6 w-6 rounded-full bg-blue-100 text-blue-600 flex items-center justify-center text-xs font-bold mr-3 mt-0.5">3</div>
                  <p className="text-sm text-gray-700">Ensure <code className="bg-gray-100 px-1 rounded">VITE_DISTRI_API_URL</code> points to your local endpoint.</p>
                </div>
              </div>
            )}
          </div>

          <div className="mt-8 pt-6 border-t border-gray-100 text-center">
            <p className="text-xs text-gray-500">
              Need help? Check our <a href="https://docs.distri.dev" target="_blank" rel="noopener noreferrer" className="text-blue-600 hover:underline">documentation</a> or join our Slack.
            </p>
          </div>
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