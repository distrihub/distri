import React, { useState, useEffect } from 'react';
import { MessageSquare, Settings, Activity, Loader2, Plus, Bot } from 'lucide-react';
import Chat from './components/Chat';
import AgentList from './components/AgentList';
import TaskMonitor from './components/TaskMonitor';

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
      const threadList = await response.json();
      setThreads(threadList);
      
      // Select the first thread if none is selected
      if (threadList.length > 0 && !selectedThread) {
        setSelectedThread(threadList[0]);
      }
    } catch (error) {
      console.error('Failed to fetch threads:', error);
    }
  };

  const createNewThread = async () => {
    if (!selectedAgent || creatingThread) return;

    setCreatingThread(true);
    try {
      const response = await fetch('/api/v1/threads', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          agent_id: selectedAgent.id,
          title: 'New conversation',
        }),
      });

      if (response.ok) {
        const newThread = await response.json();
        const threadSummary: Thread = {
          id: newThread.id,
          title: newThread.title,
          agent_id: newThread.agent_id,
          agent_name: selectedAgent.name,
          updated_at: newThread.updated_at,
          message_count: newThread.message_count,
          last_message: newThread.last_message,
        };
        
        setThreads((prev: Thread[]) => [threadSummary, ...prev]);
        setSelectedThread(threadSummary);
      }
    } catch (error) {
      console.error('Failed to create thread:', error);
    } finally {
      setCreatingThread(false);
    }
  };

  const updateThreadTitle = async (threadId: string, newTitle: string) => {
    try {
      const response = await fetch(`/api/v1/threads/${threadId}`, {
        method: 'PUT',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          title: newTitle,
        }),
      });

      if (response.ok) {
        setThreads((prev: Thread[]) => prev.map((thread: Thread) => 
          thread.id === threadId ? { ...thread, title: newTitle } : thread
        ));
        if (selectedThread?.id === threadId) {
          setSelectedThread((prev: Thread | null) => prev ? { ...prev, title: newTitle } : null);
        }
      }
    } catch (error) {
      console.error('Failed to update thread title:', error);
    }
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
    <div className="min-h-screen bg-gray-50">
      {/* Header */}
      <header className="bg-white shadow-sm border-b">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
          <div className="flex justify-between items-center py-4">
            <div className="flex items-center space-x-4">
              <h1 className="text-2xl font-bold text-gray-900">Distri</h1>
              <span className="text-sm text-gray-500">AI Agent Platform</span>
            </div>
            
            {/* Agent Selector */}
            <div className="flex items-center space-x-4">
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
              
              <div className="flex bg-gray-100 rounded-lg p-1">
                <button
                  onClick={() => setActiveTab('chat')}
                  className={`flex items-center space-x-1 px-3 py-1 rounded text-sm font-medium transition-colors ${
                    activeTab === 'chat'
                      ? 'bg-white text-blue-600 shadow-sm'
                      : 'text-gray-600 hover:text-gray-900'
                  }`}
                >
                  <MessageSquare className="h-4 w-4" />
                  <span>Chat</span>
                </button>
                <button
                  onClick={() => setActiveTab('agents')}
                  className={`flex items-center space-x-1 px-3 py-1 rounded text-sm font-medium transition-colors ${
                    activeTab === 'agents'
                      ? 'bg-white text-blue-600 shadow-sm'
                      : 'text-gray-600 hover:text-gray-900'
                  }`}
                >
                  <Settings className="h-4 w-4" />
                  <span>Agents</span>
                </button>
                <button
                  onClick={() => setActiveTab('tasks')}
                  className={`flex items-center space-x-1 px-3 py-1 rounded text-sm font-medium transition-colors ${
                    activeTab === 'tasks'
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
      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <div className="grid grid-cols-1 lg:grid-cols-4 gap-8">
          {/* Sidebar - Threads */}
          <div className="lg:col-span-1">
            <div className="bg-white rounded-lg shadow p-4">
              <div className="flex items-center justify-between mb-4">
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
              
              {threads.length === 0 ? (
                <div className="text-center py-8">
                  <MessageSquare className="h-12 w-12 text-gray-400 mx-auto mb-4" />
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
                      className={`p-3 rounded-lg cursor-pointer transition-colors border ${
                        selectedThread?.id === thread.id
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

          {/* Main Content Area */}
          <div className="lg:col-span-3">
            {activeTab === 'chat' && selectedThread && selectedAgent && (
              <Chat 
                thread={selectedThread} 
                agent={selectedAgent}
                onThreadUpdate={fetchThreads}
              />
            )}

            {activeTab === 'chat' && !selectedThread && (
              <div className="bg-white rounded-lg shadow p-12 text-center">
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
              <AgentList agents={agents} onRefresh={fetchAgents} />
            )}

            {activeTab === 'tasks' && (
              <TaskMonitor />
            )}
          </div>
        </div>
      </main>
    </div>
  );
}

export default App;