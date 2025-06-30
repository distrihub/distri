import React from 'react';
import { X, Activity, Clock, CheckCircle, XCircle, AlertCircle, Loader2, Bot, User } from 'lucide-react';
import MessageRenderer from './MessageRenderer';

interface TaskMessage {
  messageId: string;
  role: 'user' | 'agent';
  parts: Array<{ kind: string; text?: string; [key: string]: any }>;
  metadata?: any;
}

interface TaskStatus {
  state: 'submitted' | 'working' | 'completed' | 'failed' | 'canceled';
  message?: TaskMessage;
  timestamp?: string;
}

interface Task {
  id: string;
  kind: string;
  contextId: string;
  status: TaskStatus;
  artifacts: any[];
  history: TaskMessage[];
}

interface TaskDetailsDialogProps {
  task: Task;
  isOpen: boolean;
  onClose: () => void;
}

const TaskDetailsDialog: React.FC<TaskDetailsDialogProps> = ({ 
  task, 
  isOpen, 
  onClose 
}) => {
  const [taskDetails, setTaskDetails] = React.useState<Task | null>(null);
  const [loading, setLoading] = React.useState(false);

  React.useEffect(() => {
    if (isOpen && task) {
      fetchTaskDetails();
    }
  }, [isOpen, task]);

  const fetchTaskDetails = async () => {
    setLoading(true);
    try {
      const response = await fetch(`/api/v1/tasks/${task.id}`);
      if (response.ok) {
        const details = await response.json();
        setTaskDetails(details);
      } else {
        setTaskDetails(task); // Fallback to provided task
      }
    } catch (error) {
      console.error('Failed to fetch task details:', error);
      setTaskDetails(task); // Fallback to provided task
    } finally {
      setLoading(false);
    }
  };

  const getStatusIcon = (state: string) => {
    switch (state) {
      case 'completed':
        return <CheckCircle className="h-5 w-5 text-green-500" />;
      case 'failed':
        return <XCircle className="h-5 w-5 text-red-500" />;
      case 'working':
        return <Loader2 className="h-5 w-5 text-blue-500 animate-spin" />;
      case 'canceled':
        return <AlertCircle className="h-5 w-5 text-orange-500" />;
      default:
        return <Clock className="h-5 w-5 text-gray-500" />;
    }
  };

  const getStatusColor = (state: string) => {
    switch (state) {
      case 'completed':
        return 'text-green-600 bg-green-50 border-green-200';
      case 'failed':
        return 'text-red-600 bg-red-50 border-red-200';
      case 'working':
        return 'text-blue-600 bg-blue-50 border-blue-200';
      case 'canceled':
        return 'text-orange-600 bg-orange-50 border-orange-200';
      default:
        return 'text-gray-600 bg-gray-50 border-gray-200';
    }
  };

  const formatTimestamp = (timestamp: string) => {
    const date = new Date(timestamp);
    return date.toLocaleString();
  };

  if (!isOpen) return null;

  const currentTask = taskDetails || task;

  return (
    <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50 p-4">
      <div className="bg-white rounded-lg shadow-xl max-w-4xl w-full max-h-[90vh] overflow-y-auto">
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b">
          <div className="flex items-center space-x-3">
            {getStatusIcon(currentTask.status.state)}
            <div>
              <h2 className="text-xl font-semibold text-gray-900">
                Task {currentTask.id}
              </h2>
              <div className="flex items-center space-x-2 mt-1">
                <span
                  className={`inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium border ${getStatusColor(
                    currentTask.status.state
                  )}`}
                >
                  {currentTask.status.state}
                </span>
                <span className="text-sm text-gray-500">
                  Type: {currentTask.kind}
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
              <Activity className="h-12 w-12 text-gray-400 mx-auto mb-4 animate-pulse" />
              <p className="text-gray-500">Loading task details...</p>
            </div>
          ) : (
            <div className="space-y-6">
              {/* Basic Info */}
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                <div>
                  <h3 className="text-sm font-medium text-gray-500">Task ID</h3>
                  <p className="font-mono text-sm text-gray-900">{currentTask.id}</p>
                </div>
                <div>
                  <h3 className="text-sm font-medium text-gray-500">Context ID</h3>
                  <p className="font-mono text-sm text-gray-900">{currentTask.contextId}</p>
                </div>
                <div>
                  <h3 className="text-sm font-medium text-gray-500">Type</h3>
                  <p className="text-sm text-gray-900">{currentTask.kind}</p>
                </div>
                {currentTask.status.timestamp && (
                  <div>
                    <h3 className="text-sm font-medium text-gray-500">Last Updated</h3>
                    <p className="text-sm text-gray-900">
                      {formatTimestamp(currentTask.status.timestamp)}
                    </p>
                  </div>
                )}
              </div>

              {/* Current Status Message */}
              {currentTask.status.message && (
                <div>
                  <h3 className="text-lg font-medium text-gray-900 mb-3">Current Status</h3>
                  <div className="bg-gray-50 rounded-lg p-4">
                    <div className="flex items-start space-x-3">
                      {currentTask.status.message.role === 'agent' ? (
                        <Bot className="h-5 w-5 text-gray-600 mt-0.5 flex-shrink-0" />
                      ) : (
                        <User className="h-5 w-5 text-gray-600 mt-0.5 flex-shrink-0" />
                      )}
                      <div className="flex-1">
                        <MessageRenderer
                          content={currentTask.status.message.parts
                            .filter(part => part.kind === 'text')
                            .map(part => part.text || '')
                            .join('')}
                          className="text-gray-700"
                        />
                      </div>
                    </div>
                  </div>
                </div>
              )}

              {/* Artifacts */}
              {currentTask.artifacts && currentTask.artifacts.length > 0 && (
                <div>
                  <h3 className="text-lg font-medium text-gray-900 mb-3">
                    Artifacts ({currentTask.artifacts.length})
                  </h3>
                  <div className="space-y-3">
                    {currentTask.artifacts.map((artifact, index) => (
                      <div key={index} className="border border-gray-200 rounded-lg p-4">
                        <div className="flex items-start justify-between mb-2">
                          <h4 className="font-medium text-gray-900">
                            {artifact.name || `Artifact ${index + 1}`}
                          </h4>
                          <span className="text-xs text-gray-500">
                            ID: {artifact.artifactId || 'N/A'}
                          </span>
                        </div>
                        {artifact.description && (
                          <p className="text-sm text-gray-600 mb-2">{artifact.description}</p>
                        )}
                        {artifact.parts && artifact.parts.length > 0 && (
                          <div className="space-y-2">
                            {artifact.parts.map((part: any, partIndex: number) => (
                              <div key={partIndex} className="bg-gray-50 rounded p-2">
                                <MessageRenderer
                                  content={part.text || JSON.stringify(part, null, 2)}
                                  className="text-sm"
                                />
                              </div>
                            ))}
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              )}

              {/* History */}
              {currentTask.history && currentTask.history.length > 0 && (
                <div>
                  <h3 className="text-lg font-medium text-gray-900 mb-3">
                    History ({currentTask.history.length} messages)
                  </h3>
                  <div className="space-y-4 max-h-96 overflow-y-auto">
                    {currentTask.history.map((message, index) => (
                      <div
                        key={message.messageId || index}
                        className={`flex ${message.role === 'user' ? 'justify-end' : 'justify-start'}`}
                      >
                        <div
                          className={`max-w-[80%] rounded-lg px-4 py-3 ${
                            message.role === 'user'
                              ? 'bg-blue-600 text-white'
                              : 'bg-gray-100 text-gray-900'
                          }`}
                        >
                          <div className="flex items-start space-x-2">
                            {message.role === 'agent' && (
                              <Bot className="h-4 w-4 mt-0.5 flex-shrink-0" />
                            )}
                            {message.role === 'user' && (
                              <User className="h-4 w-4 mt-0.5 flex-shrink-0" />
                            )}
                            <div className="flex-1">
                              <MessageRenderer
                                content={message.parts
                                  .filter(part => part.kind === 'text')
                                  .map(part => part.text || '')
                                  .join('')}
                                className={message.role === 'user' ? 'text-white' : ''}
                              />
                              {message.metadata?.timestamp && (
                                <p className={`text-xs mt-1 ${
                                  message.role === 'user' ? 'text-blue-200' : 'text-gray-500'
                                }`}>
                                  {formatTimestamp(message.metadata.timestamp)}
                                </p>
                              )}
                            </div>
                          </div>
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              )}

              {/* Empty states */}
              {(!currentTask.history || currentTask.history.length === 0) && 
               (!currentTask.artifacts || currentTask.artifacts.length === 0) && 
               !currentTask.status.message && (
                <div className="text-center py-8">
                  <Activity className="h-12 w-12 text-gray-400 mx-auto mb-4" />
                  <p className="text-gray-500">No additional details available for this task</p>
                </div>
              )}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end p-6 border-t bg-gray-50">
          <button
            onClick={onClose}
            className="px-4 py-2 text-gray-600 hover:text-gray-800"
          >
            Close
          </button>
        </div>
      </div>
    </div>
  );
};

export default TaskDetailsDialog;