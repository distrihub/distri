import React, { useState, useEffect } from 'react';
import { MessageSquare, Settings, Activity, Loader2, Globe, Shield, Zap } from 'lucide-react';
import Chat from './components/Chat';
import AgentList from './components/AgentList';
import TaskMonitor from './components/TaskMonitor';

interface AgentCapabilities {
  streaming: boolean;
  pushNotifications: boolean;
  stateTransitionHistory: boolean;
  extensions: any[];
}

interface AgentSkill {
  id: string;
  name: string;
  description: string;
  tags: string[];
  examples: string[];
  inputModes?: string[];
  outputModes?: string[];
}

interface AgentCard {
  version: string;
  name: string;
  description: string;
  url: string;
  iconUrl?: string;
  documentationUrl?: string;
  capabilities: AgentCapabilities;
  defaultInputModes: string[];
  defaultOutputModes: string[];
  skills: AgentSkill[];
  provider?: {
    organization: string;
    url: string;
  };
}

interface Agent {
  id: string;
  name: string;
  description: string;
  status: 'online' | 'offline';
  card?: AgentCard;
  skills?: AgentSkill[];
  capabilities?: AgentCapabilities;
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
      // Use A2A .well-known endpoint for agent discovery
      const response = await fetch('/.well-known/agent-cards');
      const agentCards: AgentCard[] = await response.json();

      const formattedAgents: Agent[] = agentCards.map((card) => ({
        id: card.name,
        name: card.name,
        description: card.description,
        status: 'online' as const,
        card,
        skills: card.skills,
        capabilities: card.capabilities,
      }));

      setAgents(formattedAgents);
      if (formattedAgents.length > 0 && !selectedAgent) {
        setSelectedAgent(formattedAgents[0]);
      }
    } catch (error) {
      console.error('Failed to fetch agent cards:', error);
      // Fallback to legacy API
      try {
        const fallbackResponse = await fetch('/api/v1/agents');
        const legacyAgents = await fallbackResponse.json();
        const formattedAgents: Agent[] = legacyAgents.map((agent: any) => ({
          id: agent.name,
          name: agent.name,
          description: agent.description,
          status: 'online' as const,
        }));
        setAgents(formattedAgents);
        if (formattedAgents.length > 0 && !selectedAgent) {
          setSelectedAgent(formattedAgents[0]);
        }
      } catch (fallbackError) {
        console.error('Failed to fetch agents from fallback API:', fallbackError);
      }
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

  const renderAgentCapabilities = (agent: Agent) => {
    if (!agent.capabilities) return null;

    return (
      <div className="mb-4 p-3 bg-blue-50 rounded-lg">
        <h4 className="text-sm font-medium text-blue-900 mb-2 flex items-center">
          <Zap className="h-4 w-4 mr-1" />
          Capabilities
        </h4>
        <div className="flex flex-wrap gap-2">
          {agent.capabilities.streaming && (
            <span className="inline-flex items-center px-2 py-1 rounded-full text-xs bg-green-100 text-green-800">
              <Globe className="h-3 w-3 mr-1" />
              Streaming
            </span>
          )}
          {agent.capabilities.pushNotifications && (
            <span className="inline-flex items-center px-2 py-1 rounded-full text-xs bg-blue-100 text-blue-800">
              Push Notifications
            </span>
          )}
          {agent.capabilities.stateTransitionHistory && (
            <span className="inline-flex items-center px-2 py-1 rounded-full text-xs bg-purple-100 text-purple-800">
              State History
            </span>
          )}
        </div>
      </div>
    );
  };

  const renderAgentSkills = (agent: Agent) => {
    if (!agent.skills || agent.skills.length === 0) return null;

    return (
      <div className="mb-4">
        <h4 className="text-sm font-medium text-gray-700 mb-2">Skills & Capabilities</h4>
        <div className="space-y-2">
          {agent.skills.slice(0, 3).map((skill) => (
            <div key={skill.id} className="p-2 bg-gray-50 rounded text-xs">
              <div className="font-medium text-gray-900">{skill.name}</div>
              <div className="text-gray-600 mt-1">{skill.description}</div>
              {skill.tags.length > 0 && (
                <div className="flex flex-wrap gap-1 mt-1">
                  {skill.tags.map((tag) => (
                    <span key={tag} className="px-1 py-0.5 bg-gray-200 rounded text-xs text-gray-600">
                      {tag}
                    </span>
                  ))}
                </div>
              )}
            </div>
          ))}
          {agent.skills.length > 3 && (
            <div className="text-xs text-gray-500 text-center">
              +{agent.skills.length - 3} more skills
            </div>
          )}
        </div>
      </div>
    );
  };

  return (
    <div className="min-h-screen bg-gray-50">
      {/* Header */}
      <header className="bg-white shadow-sm border-b">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
          <div className="flex justify-between items-center py-4">
            <div className="flex items-center space-x-4">
              <h1 className="text-2xl font-bold text-gray-900">Distri</h1>
              <span className="text-sm text-gray-500">A2A-Compatible Agent Platform</span>
              {selectedAgent?.card && (
                <span className="inline-flex items-center px-2 py-1 rounded-full text-xs bg-green-100 text-green-800">
                  <Shield className="h-3 w-3 mr-1" />
                  A2A v{selectedAgent.card.version}
                </span>
              )}
            </div>
            <div className="flex items-center space-x-2">
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
      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <div className="grid grid-cols-1 lg:grid-cols-4 gap-8">
          {/* Enhanced Sidebar */}
          <div className="lg:col-span-1">
            <div className="bg-white rounded-lg shadow p-6">
              <h2 className="text-lg font-medium text-gray-900 mb-4">Available Agents</h2>
              <div className="space-y-2">
                {agents.map((agent) => (
                  <button
                    key={agent.id}
                    onClick={() => setSelectedAgent(agent)}
                    className={`w-full text-left p-3 rounded-lg transition-colors ${selectedAgent?.id === agent.id
                      ? 'bg-blue-50 border border-blue-200'
                      : 'hover:bg-gray-50 border border-transparent'
                      }`}
                  >
                    <div className="flex items-start justify-between">
                      <div className="flex-1">
                        <div className="flex items-center space-x-2">
                          {agent.card?.iconUrl ? (
                            <img
                              src={agent.card.iconUrl}
                              alt={`${agent.name} icon`}
                              className="w-6 h-6 rounded"
                            />
                          ) : (
                            <div className="w-6 h-6 bg-blue-500 rounded flex items-center justify-center text-white text-xs font-bold">
                              {agent.name.charAt(0).toUpperCase()}
                            </div>
                          )}
                          <h3 className="font-medium text-gray-900">{agent.name}</h3>
                        </div>
                        <p className="text-sm text-gray-500 mt-1 truncate">{agent.description}</p>

                        {/* Agent Provider */}
                        {agent.card?.provider && (
                          <div className="text-xs text-gray-400 mt-1">
                            by {agent.card.provider.organization}
                          </div>
                        )}
                      </div>
                      <div className={`w-2 h-2 rounded-full flex-shrink-0 ${agent.status === 'online' ? 'bg-green-400' : 'bg-gray-400'
                        }`} />
                    </div>

                    {selectedAgent?.id === agent.id && (
                      <div className="mt-3 space-y-3">
                        {renderAgentCapabilities(agent)}
                        {renderAgentSkills(agent)}
                      </div>
                    )}
                  </button>
                ))}
              </div>
            </div>
          </div>

          {/* Main Content Area */}
          <div className="lg:col-span-3">
            {activeTab === 'chat' && selectedAgent && (
              <Chat agent={selectedAgent} />
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