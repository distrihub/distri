import { useState, useEffect } from 'react';
import { MessageSquare, Settings, Activity, Loader2, Plus, Bot } from 'lucide-react';
import Chat from './components/Chat';
import AgentList from './components/AgentList';
import TaskMonitor from './components/TaskMonitor';
import { v4 as uuidv4 } from 'uuid';

interface Agent {
  id: string;
  name: string;
  description: string;
  status: 'online' | 'offline';
}

interface Thread {
  id: string;
  title: string;
  agent_id: string;
  agent_name: string;
  updated_at: string;
  message_count: number;
  last_message?: string;
}

function App() {
  const [selectedThread, setSelectedThread] = useState<Thread | null>(null);
  const [selectedAgent, setSelectedAgent] = useState<Agent | null>(null);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [threads, setThreads] = useState<Thread[]>([]);
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<'chat' | 'agents' | 'tasks'>('chat');
  const [creatingThread, setCreatingThread] = useState(false);

  useEffect(() => {
    Promise.all([fetchAgents(), fetchThreads()]).finally(() => setLoading(false));
  }, []);

  const fetchAgents = async () => {
    try {
      const response = await fetch('/api/v1/agents');
      const agentCards = await response.json();

      const formattedAgents: Agent[] = agentCards.map((card: any) => ({
        id: card.name,
        name: card.name,
        description: card.description,
        status: 'online' as const,
      }));

      setAgents(formattedAgents);
      if (formattedAgents.length > 0 && !selectedAgent) {
        setSelectedAgent(formattedAgents[0]);
      }
    } catch (error) {
      console.error('Failed to fetch agents:', error);
    }
  };

  const fetchThreads = async () => {
    try {
      const response = await fetch('/api/v1/threads');
      if (response.ok) {
        const threadList = await response.json();

        // Merge server threads with any local threads that may not be persisted yet
        setThreads((currentThreads: Thread[]) => {
          const serverThreadIds = new Set(threadList.map((t: Thread) => t.id));
          const localThreads = currentThreads.filter(t => !serverThreadIds.has(t.id));

          // Combine server threads with local threads, server threads first
          return [...threadList, ...localThreads];
        });

        // Select the first thread if none is selected
        if (threadList.length > 0 && !selectedThread) {
          setSelectedThread(threadList[0]);
        }
      }
    } catch (error) {
      console.error('Failed to fetch threads:', error);
      // If we can't fetch from server, keep the current threads (likely local ones)
    }
  };

  const createNewThread = async () => {
    if (!selectedAgent || creatingThread) return;

    setCreatingThread(true);
    try {
      // Create thread summary locally first
      const newThreadId = uuidv4();
      const threadSummary: Thread = {
        id: newThreadId,
        title: 'New conversation',
        agent_id: selectedAgent.id,
        agent_name: selectedAgent.name,
        updated_at: new Date().toISOString(),
        message_count: 0,
        last_message: undefined,
      };

      // Add to local state immediately for better UX
      setThreads((prev: Thread[]) => [threadSummary, ...prev]);
      setSelectedThread(threadSummary);

      // The actual thread will be created in the backend when the first message is sent
      // with the thread.id as contextId, ensuring the backend uses our thread ID
    } catch (error) {
      console.error('Failed to create thread:', error);
    } finally {
      setCreatingThread(false);
    }
  };

  const startChatWithAgent = async (agent: Agent) => {
    // Set the selected agent
    setSelectedAgent(agent);

    // Switch to chat tab
    setActiveTab('chat');

    // Create a new thread for this agent
    const newThreadId = uuidv4();
    const threadSummary: Thread = {
      id: newThreadId,
      title: `Chat with ${agent.name}`,
      agent_id: agent.id,
      agent_name: agent.name,
      updated_at: new Date().toISOString(),
      message_count: 0,
      last_message: undefined,
    };

    // Add to local state immediately for better UX
    setThreads((prev: Thread[]) => [threadSummary, ...prev]);
    setSelectedThread(threadSummary);
  };

  const deleteThread = async (threadId: string) => {
    try {
      const response = await fetch(`/api/v1/threads/${threadId}`, {
        method: 'DELETE',
      });

      if (response.ok) {
        setThreads((prev: Thread[]) => prev.filter((thread: Thread) => thread.id !== threadId));
        if (selectedThread?.id === threadId) {
          const remainingThreads = threads.filter(thread => thread.id !== threadId);
          setSelectedThread(remainingThreads.length > 0 ? remainingThreads[0] : null);
        }
      }
    } catch (error) {
      console.error('Failed to delete thread:', error);
    }
  };

  const updateSpecificThread = async (threadId: string) => {
    try {
      const response = await fetch(`/api/v1/threads/${threadId}`);
      if (response.ok) {
        const updatedThread = await response.json();
        const threadSummary: Thread = {
          id: updatedThread.id,
          title: updatedThread.title,
          agent_id: updatedThread.agent_id,
          agent_name: agents.find((a: Agent) => a.id === updatedThread.agent_id)?.name || updatedThread.agent_id,
          updated_at: updatedThread.updated_at,
          message_count: updatedThread.message_count,
          last_message: updatedThread.last_message,
        };

        setThreads((prev: Thread[]) =>
          prev.map((thread: Thread) =>
            thread.id === threadId ? threadSummary : thread
          )
        );

        if (selectedThread?.id === threadId) {
          setSelectedThread(threadSummary);
        }
      }
    } catch (error) {
      console.error('Failed to update thread:', error);
    }
  };

  if (loading) {
    return (
      <div className="min-h-screen bg-gray-50 flex items-center justify-center">
        <div className="flex items-center space-x-2">
          <Loader2 className="h-6 w-6 animate-spin text-blue-600" />
          <span className="text-gray-600">Loading...</span>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-50 flex flex-col">
      {/* Header */}
      <header className="bg-white shadow-sm border-b flex-shrink-0">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
          <div className="flex justify-between items-center py-4">
            <div className="flex items-center space-x-4">
              <button
                onClick={() => setActiveTab('chat')}
                className="flex items-center space-x-4 hover:opacity-80 transition-opacity"
              >
                <h1 className="text-2xl font-bold text-gray-900">Distri</h1>
                <span className="text-sm text-gray-500">AI Agent Platform</span>
              </button>
            </div>

            {/* Agent Selector - only show on chat tab */}
            <div className="flex items-center space-x-4">
              {activeTab === 'chat' && (
                <div className="flex items-center space-x-2">
                  <Bot className="h-4 w-4 text-gray-600" />
                  <select
                    value={selectedAgent?.id || ''}
                    onChange={(e) => {
                      const agent = agents.find(a => a.id === e.target.value);
                      setSelectedAgent(agent || null);
                    }}
                    className="border border-gray-300 rounded-md px-3 py-1 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                  >
                    {agents.map((agent) => (
                      <option key={agent.id} value={agent.id}>
                        {agent.name}
                      </option>
                    ))}
                  </select>
                </div>
              )}

              <div className="flex bg-gray-100 rounded-lg p-1">
                <button
                  onClick={() => setActiveTab('chat')}
                  className={`flex items-center space-x-1 px-3 py-1 rounded text-sm font-medium transition-colors ${activeTab === 'chat'
                    ? 'bg-white text-blue-600 shadow-sm'
                    : 'text-gray-600 hover:text-gray-900'
                    }`}
                >
                  <MessageSquare className="h-4 w-4" />
                  <span>Chat</span>
                </button>
                <button
                  onClick={() => setActiveTab('agents')}
                  className={`flex items-center space-x-1 px-3 py-1 rounded text-sm font-medium transition-colors ${activeTab === 'agents'
                    ? 'bg-white text-blue-600 shadow-sm'
                    : 'text-gray-600 hover:text-gray-900'
                    }`}
                >
                  <Settings className="h-4 w-4" />
                  <span>Agents</span>
                </button>
                <button
                  onClick={() => setActiveTab('tasks')}
                  className={`flex items-center space-x-1 px-3 py-1 rounded text-sm font-medium transition-colors ${activeTab === 'tasks'
                    ? 'bg-white text-blue-600 shadow-sm'
                    : 'text-gray-600 hover:text-gray-900'
                    }`}
                >
                  <Activity className="h-4 w-4" />
                  <span>Tasks</span>
                </button>
              </div>
            </div>
          </div>
        </div>
      </header>

      {/* Main Content */}
      <main className="flex-1 flex max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8 w-full">
        <div className={`flex-1 flex gap-8 ${activeTab === 'chat' ? '' : 'justify-center'}`}>
          {/* Sidebar - Threads (only show on chat tab) */}
          {activeTab === 'chat' && (
            <div className="w-80 flex-shrink-0">
              <div className="bg-white rounded-lg shadow p-4 h-full flex flex-col">
                <div className="flex items-center justify-between mb-4 flex-shrink-0">
                  <h2 className="text-lg font-medium text-gray-900">Conversations</h2>
                  <button
                    onClick={createNewThread}
                    disabled={!selectedAgent || creatingThread}
                    className="flex items-center space-x-1 px-3 py-1 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
                  >
                    {creatingThread ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : (
                      <Plus className="h-4 w-4" />
                    )}
                    <span>New</span>
                  </button>
                </div>

                <div className="flex-1 overflow-y-auto">
                  {threads.length === 0 ? (
                    <div className="text-center py-8">
                      <p className="text-gray-500 text-sm">No conversations yet</p>
                      <p className="text-gray-400 text-xs mt-1">
                        {selectedAgent ? 'Click "New" to start' : 'Select an agent first'}
                      </p>
                    </div>
                  ) : (
                    <div className="space-y-2">
                      {threads.map((thread) => (
                        <div
                          key={thread.id}
                          onClick={() => setSelectedThread(thread)}
                          className={`p-3 rounded-lg cursor-pointer transition-colors border ${selectedThread?.id === thread.id
                            ? 'bg-blue-50 border-blue-200'
                            : 'hover:bg-gray-50 border-transparent'
                            }`}
                        >
                          <div className="flex items-start justify-between">
                            <div className="flex-1 min-w-0">
                              <h3 className="font-medium text-gray-900 text-sm truncate">
                                {thread.title}
                              </h3>
                              <p className="text-xs text-gray-500 mt-1">
                                with {thread.agent_name}
                              </p>
                              {thread.last_message && (
                                <p className="text-xs text-gray-400 mt-1 truncate">
                                  {thread.last_message}
                                </p>
                              )}
                            </div>
                            <div className="flex flex-col items-end text-xs text-gray-400">
                              <span>{new Date(thread.updated_at).toLocaleDateString()}</span>
                              <span className="mt-1">{thread.message_count} msgs</span>
                            </div>
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            </div>
          )}

          {/* Main Content Area */}
          <div className={`flex-1 h-full ${activeTab === 'chat' ? '' : 'max-w-6xl'}`}>
            {activeTab === 'chat' && selectedThread && selectedAgent && (
              <div className="h-full">
                <Chat
                  thread={selectedThread}
                  agent={selectedAgent}
                  onThreadUpdate={() => updateSpecificThread(selectedThread.id)}
                />
              </div>
            )}

            {activeTab === 'chat' && !selectedThread && (
              <div className="bg-white rounded-lg shadow p-12 text-center h-full flex flex-col justify-center">
                <MessageSquare className="h-16 w-16 text-gray-400 mx-auto mb-4" />
                <h3 className="text-lg font-medium text-gray-900 mb-2">
                  {selectedAgent ? 'Start a conversation' : 'Select an agent'}
                </h3>
                <p className="text-gray-500">
                  {selectedAgent
                    ? 'Click "New" to create your first conversation'
                    : 'Choose an agent from the dropdown to begin'
                  }
                </p>
              </div>
            )}

            {activeTab === 'agents' && (
              <div className="h-full">
                <AgentList
                  agents={agents}
                  onRefresh={fetchAgents}
                  onStartChat={startChatWithAgent}
                />
              </div>
            )}

            {activeTab === 'tasks' && (
              <div className="h-full">
                <TaskMonitor />
              </div>
            )}
          </div>
        </div>
      </main>
    </div>
  );
}

export default App;