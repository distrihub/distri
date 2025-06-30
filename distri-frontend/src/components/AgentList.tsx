import React from 'react';
import { RefreshCw, Bot, CheckCircle, XCircle, MessageSquare } from 'lucide-react';
import AgentDetailsDialog from './AgentDetailsDialog';

interface Agent {
  id: string;
  name: string;
  description: string;
  status: 'online' | 'offline';
}

interface AgentListProps {
  agents: Agent[];
  onRefresh: () => void;
  onStartChat?: (agent: Agent) => void;
}

const AgentList: React.FC<AgentListProps> = ({ agents, onRefresh, onStartChat }) => {
  const [selectedAgent, setSelectedAgent] = React.useState<Agent | null>(null);
  const [dialogOpen, setDialogOpen] = React.useState(false);

  const handleViewDetails = (agent: Agent) => {
    setSelectedAgent(agent);
    setDialogOpen(true);
  };

  const handleCloseDialog = () => {
    setDialogOpen(false);
    setSelectedAgent(null);
  };

  const handleStartChat = (agent: Agent) => {
    if (onStartChat) {
      onStartChat(agent);
    }
    handleCloseDialog();
  };

  return (
    <>
      <div className="bg-white rounded-lg shadow">
        <div className="flex items-center justify-between p-6 border-b">
          <h2 className="text-lg font-medium text-gray-900">Agent Management</h2>
          <button
            onClick={onRefresh}
            className="flex items-center space-x-2 px-3 py-2 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors"
          >
            <RefreshCw className="h-4 w-4" />
            <span>Refresh</span>
          </button>
        </div>

        <div className="p-6">
          {agents.length === 0 ? (
            <div className="text-center py-8">
              <Bot className="h-12 w-12 text-gray-400 mx-auto mb-4" />
              <p className="text-gray-500">No agents available</p>
            </div>
          ) : (
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              {agents.map((agent) => (
                <div
                  key={agent.id}
                  className="border border-gray-200 rounded-lg p-4 hover:shadow-md transition-shadow"
                >
                  <div className="flex items-start justify-between mb-3">
                    <div className="flex items-center space-x-3">
                      <div className="w-10 h-10 bg-blue-600 rounded-lg flex items-center justify-center">
                        <Bot className="h-5 w-5 text-white" />
                      </div>
                      <div>
                        <h3 className="font-medium text-gray-900">{agent.name}</h3>
                        <div className="flex items-center space-x-1 mt-1">
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
                  </div>
                  
                  <p className="text-sm text-gray-600 mb-4">{agent.description}</p>
                  
                  <div className="flex items-center justify-between">
                    <span className="text-xs text-gray-500">ID: {agent.id}</span>
                    <div className="flex space-x-2">
                      <button
                        onClick={() => handleViewDetails(agent)}
                        className="text-sm text-blue-600 hover:text-blue-800 font-medium"
                      >
                        View Details
                      </button>
                      {onStartChat && agent.status === 'online' && (
                        <button
                          onClick={() => handleStartChat(agent)}
                          className="flex items-center space-x-1 text-sm bg-green-600 text-white px-2 py-1 rounded hover:bg-green-700"
                        >
                          <MessageSquare className="h-3 w-3" />
                          <span>Chat</span>
                        </button>
                      )}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Agent Details Dialog */}
      {selectedAgent && (
        <AgentDetailsDialog
          agent={selectedAgent}
          isOpen={dialogOpen}
          onClose={handleCloseDialog}
          onStartChat={onStartChat ? handleStartChat : undefined}
        />
      )}
    </>
  );
};

export default AgentList;