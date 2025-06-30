import React from 'react';
import { X, Bot, CheckCircle, XCircle, MessageSquare, Settings, ExternalLink } from 'lucide-react';

interface AgentCapabilities {
  streaming?: boolean;
  pushNotifications?: boolean;
  stateTransitionHistory?: boolean;
  extensions?: any[];
}

interface AgentSkill {
  id: string;
  name: string;
  description: string;
  tags: string[];
  examples?: string[];
}

interface AgentCard {
  name: string;
  description: string;
  version?: string;
  iconUrl?: string;
  documentationUrl?: string;
  url?: string;
  capabilities?: AgentCapabilities;
  defaultInputModes?: string[];
  defaultOutputModes?: string[];
  skills?: AgentSkill[];
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
}

interface AgentDetailsDialogProps {
  agent: Agent;
  isOpen: boolean;
  onClose: () => void;
  onStartChat?: (agent: Agent) => void;
}

const AgentDetailsDialog: React.FC<AgentDetailsDialogProps> = ({ 
  agent, 
  isOpen, 
  onClose, 
  onStartChat 
}) => {
  const [agentCard, setAgentCard] = React.useState<AgentCard | null>(null);
  const [loading, setLoading] = React.useState(false);

  React.useEffect(() => {
    if (isOpen && agent) {
      fetchAgentCard();
    }
  }, [isOpen, agent]);

  const fetchAgentCard = async () => {
    setLoading(true);
    try {
      const response = await fetch(`/api/v1/agents/${agent.id}`);
      if (response.ok) {
        const card = await response.json();
        setAgentCard(card);
      }
    } catch (error) {
      console.error('Failed to fetch agent card:', error);
    } finally {
      setLoading(false);
    }
  };

  // Determine agent type based on A2A capabilities
  const getAgentType = (): string => {
    if (!agentCard) return 'Custom Agent';
    
    // Check if it's a Distri agent
    if (agentCard.provider?.organization === 'Distri' || agentCard.url?.includes('distri')) {
      return 'Distri Agent';
    }
    
    // Check if it's a remote agent (has external URL)
    if (agentCard.url && !agentCard.url.includes('localhost')) {
      return 'Remote Agent';
    }
    
    return 'Custom Agent';
  };

  const getAgentTypeColor = (type: string): string => {
    switch (type) {
      case 'Distri Agent':
        return 'bg-blue-100 text-blue-800 border-blue-200';
      case 'Remote Agent':
        return 'bg-green-100 text-green-800 border-green-200';
      case 'Custom Agent':
        return 'bg-purple-100 text-purple-800 border-purple-200';
      default:
        return 'bg-gray-100 text-gray-800 border-gray-200';
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50 p-4">
      <div className="bg-white rounded-lg shadow-xl max-w-2xl w-full max-h-[90vh] overflow-y-auto">
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b">
          <div className="flex items-center space-x-3">
            <div className="w-10 h-10 bg-blue-600 rounded-lg flex items-center justify-center">
              <Bot className="h-5 w-5 text-white" />
            </div>
            <div>
              <h2 className="text-xl font-semibold text-gray-900">{agent.name}</h2>
              <div className="flex items-center space-x-2 mt-1">
                {agent.status === 'online' ? (
                  <CheckCircle className="h-4 w-4 text-green-500" />
                ) : (
                  <XCircle className="h-4 w-4 text-red-500" />
                )}
                <span className={`text-sm capitalize ${
                  agent.status === 'online' ? 'text-green-600' : 'text-red-600'
                }`}>
                  {agent.status}
                </span>
              </div>
            </div>
          </div>
          <button
            onClick={onClose}
            className="text-gray-400 hover:text-gray-600"
          >
            <X className="h-6 w-6" />
          </button>
        </div>

        {/* Content */}
        <div className="p-6">
          {loading ? (
            <div className="text-center py-8">
              <Bot className="h-12 w-12 text-gray-400 mx-auto mb-4 animate-pulse" />
              <p className="text-gray-500">Loading agent details...</p>
            </div>
          ) : (
            <div className="space-y-6">
              {/* Agent Type and Basic Info */}
              <div>
                <div className="flex items-center justify-between mb-4">
                  <span
                    className={`inline-flex items-center px-3 py-1 rounded-full text-sm font-medium border ${getAgentTypeColor(getAgentType())}`}
                  >
                    {getAgentType()}
                  </span>
                  {agentCard?.version && (
                    <span className="text-sm text-gray-500">v{agentCard.version}</span>
                  )}
                </div>
                <p className="text-gray-700 leading-relaxed">
                  {agentCard?.description || agent.description}
                </p>
              </div>

              {/* Provider Information */}
              {agentCard?.provider && (
                <div>
                  <h3 className="text-lg font-medium text-gray-900 mb-3">Provider</h3>
                  <div className="bg-gray-50 rounded-lg p-4">
                    <div className="flex items-center justify-between">
                      <span className="font-medium">{agentCard.provider.organization}</span>
                      {agentCard.provider.url && (
                        <a
                          href={agentCard.provider.url}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="text-blue-600 hover:text-blue-800 flex items-center space-x-1"
                        >
                          <ExternalLink className="h-4 w-4" />
                          <span>Visit</span>
                        </a>
                      )}
                    </div>
                  </div>
                </div>
              )}

              {/* Capabilities */}
              {agentCard?.capabilities && (
                <div>
                  <h3 className="text-lg font-medium text-gray-900 mb-3">Capabilities</h3>
                  <div className="bg-gray-50 rounded-lg p-4">
                    <div className="grid grid-cols-2 gap-4">
                      <div className="flex items-center space-x-2">
                        <div className={`w-2 h-2 rounded-full ${
                          agentCard.capabilities.streaming ? 'bg-green-400' : 'bg-gray-300'
                        }`} />
                        <span className="text-sm">Streaming</span>
                      </div>
                      <div className="flex items-center space-x-2">
                        <div className={`w-2 h-2 rounded-full ${
                          agentCard.capabilities.pushNotifications ? 'bg-green-400' : 'bg-gray-300'
                        }`} />
                        <span className="text-sm">Push Notifications</span>
                      </div>
                      <div className="flex items-center space-x-2">
                        <div className={`w-2 h-2 rounded-full ${
                          agentCard.capabilities.stateTransitionHistory ? 'bg-green-400' : 'bg-gray-300'
                        }`} />
                        <span className="text-sm">State History</span>
                      </div>
                    </div>
                  </div>
                </div>
              )}

              {/* Input/Output Modes */}
              {(agentCard?.defaultInputModes || agentCard?.defaultOutputModes) && (
                <div>
                  <h3 className="text-lg font-medium text-gray-900 mb-3">Supported Modes</h3>
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    {agentCard.defaultInputModes && (
                      <div>
                        <h4 className="font-medium text-gray-700 mb-2">Input</h4>
                        <div className="flex flex-wrap gap-2">
                          {agentCard.defaultInputModes.map((mode, index) => (
                            <span
                              key={index}
                              className="px-2 py-1 bg-blue-100 text-blue-800 text-xs rounded-full"
                            >
                              {mode}
                            </span>
                          ))}
                        </div>
                      </div>
                    )}
                    {agentCard.defaultOutputModes && (
                      <div>
                        <h4 className="font-medium text-gray-700 mb-2">Output</h4>
                        <div className="flex flex-wrap gap-2">
                          {agentCard.defaultOutputModes.map((mode, index) => (
                            <span
                              key={index}
                              className="px-2 py-1 bg-green-100 text-green-800 text-xs rounded-full"
                            >
                              {mode}
                            </span>
                          ))}
                        </div>
                      </div>
                    )}
                  </div>
                </div>
              )}

              {/* Skills */}
              {agentCard?.skills && agentCard.skills.length > 0 && (
                <div>
                  <h3 className="text-lg font-medium text-gray-900 mb-3">Skills</h3>
                  <div className="space-y-4">
                    {agentCard.skills.map((skill) => (
                      <div key={skill.id} className="border border-gray-200 rounded-lg p-4">
                        <div className="flex items-start justify-between mb-2">
                          <h4 className="font-medium text-gray-900">{skill.name}</h4>
                          <div className="flex flex-wrap gap-1">
                            {skill.tags.map((tag, index) => (
                              <span
                                key={index}
                                className="px-2 py-1 bg-gray-100 text-gray-600 text-xs rounded-full"
                              >
                                {tag}
                              </span>
                            ))}
                          </div>
                        </div>
                        <p className="text-sm text-gray-600 mb-2">{skill.description}</p>
                        {skill.examples && skill.examples.length > 0 && (
                          <div>
                            <p className="text-xs font-medium text-gray-500 mb-1">Examples:</p>
                            <ul className="text-xs text-gray-500 list-disc list-inside">
                              {skill.examples.map((example, index) => (
                                <li key={index}>{example}</li>
                              ))}
                            </ul>
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              )}

              {/* Documentation Link */}
              {agentCard?.documentationUrl && (
                <div>
                  <a
                    href={agentCard.documentationUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="inline-flex items-center space-x-2 text-blue-600 hover:text-blue-800"
                  >
                    <ExternalLink className="h-4 w-4" />
                    <span>View Documentation</span>
                  </a>
                </div>
              )}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between p-6 border-t bg-gray-50">
          <div className="text-sm text-gray-500">
            ID: {agent.id}
          </div>
          <div className="flex space-x-3">
            <button
              onClick={onClose}
              className="px-4 py-2 text-gray-600 hover:text-gray-800"
            >
              Close
            </button>
            {onStartChat && (
              <button
                onClick={() => onStartChat(agent)}
                className="flex items-center space-x-2 px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700"
              >
                <MessageSquare className="h-4 w-4" />
                <span>Chat with {agent.name}</span>
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

export default AgentDetailsDialog;