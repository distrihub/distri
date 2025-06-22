import React, { useState, useEffect } from 'react';
import { AgentProvider, useAgent } from '@ag-ui/react';
import { MessageSquare, Settings, Activity, Loader2 } from 'lucide-react';
import Chat from './components/Chat';
import AgentList from './components/AgentList';
import TaskMonitor from './components/TaskMonitor';

interface Agent {
  id: string;
  name: string;
  description: string;
  status: 'online' | 'offline';
}

function App() {
  const [selectedAgent, setSelectedAgent] = useState<Agent | null>(null);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<'chat' | 'agents' | 'tasks'>('chat');

  useEffect(() => {
    fetchAgents();
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
    } finally {
      setLoading(false);
    }
  };

  if (loading) {
    return (
      <div className="min-h-screen bg-gray-50 flex items-center justify-center">
        <div className="flex items-center space-x-2">
          <Loader2 className="h-6 w-6 animate-spin text-blue-600" />
          <span className="text-gray-600">Loading agents...</span>
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
            <div className="flex items-center space-x-2">
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
          {/* Sidebar */}
          <div className="lg:col-span-1">
            <div className="bg-white rounded-lg shadow p-6">
              <h2 className="text-lg font-medium text-gray-900 mb-4">Available Agents</h2>
              <div className="space-y-2">
                {agents.map((agent) => (
                  <button
                    key={agent.id}
                    onClick={() => setSelectedAgent(agent)}
                    className={`w-full text-left p-3 rounded-lg transition-colors ${
                      selectedAgent?.id === agent.id
                        ? 'bg-blue-50 border border-blue-200'
                        : 'hover:bg-gray-50 border border-transparent'
                    }`}
                  >
                    <div className="flex items-center justify-between">
                      <div>
                        <h3 className="font-medium text-gray-900">{agent.name}</h3>
                        <p className="text-sm text-gray-500 truncate">{agent.description}</p>
                      </div>
                      <div className={`w-2 h-2 rounded-full ${
                        agent.status === 'online' ? 'bg-green-400' : 'bg-gray-400'
                      }`} />
                    </div>
                  </button>
                ))}
              </div>
            </div>
          </div>

          {/* Main Content Area */}
          <div className="lg:col-span-3">
            {activeTab === 'chat' && selectedAgent && (
              <AgentProvider
                key={selectedAgent.id}
                agent={{
                  name: selectedAgent.name,
                  baseUrl: '/api/v1',
                  agentId: selectedAgent.id,
                }}
              >
                <Chat agent={selectedAgent} />
              </AgentProvider>
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