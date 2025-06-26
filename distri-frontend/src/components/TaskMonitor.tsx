import React, { useState, useEffect } from 'react';
import { Activity, Clock, CheckCircle, XCircle, AlertCircle, Loader2, FileText } from 'lucide-react';
import ArtifactRenderer from './ArtifactRenderer';

interface Artifact {
  artifactId: string;
  name?: string;
  description?: string;
  parts: Array<{
    kind: string;
    text?: string;
    data?: any;
  }>;
}

interface Task {
  id: string;
  kind: string;
  contextId: string;
  status: {
    state: 'submitted' | 'working' | 'completed' | 'failed' | 'canceled';
    message?: any;
    timestamp?: string;
  };
  artifacts: Artifact[];
  history: any[];
}

const TaskMonitor: React.FC = () => {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetchTasks();
    const interval = setInterval(fetchTasks, 5000); // Refresh every 5 seconds
    return () => clearInterval(interval);
  }, []);

  const fetchTasks = async () => {
    try {
      // This is a placeholder - in a real implementation, you'd have an endpoint to list tasks
      // For now, we'll just show a mock task list
      setTasks([
        {
          id: 'task-1',
          kind: 'message',
          contextId: 'chat-agent1',
          status: {
            state: 'completed',
            timestamp: new Date(Date.now() - 1000 * 60 * 5).toISOString(),
          },
          artifacts: [],
          history: [],
        },
        {
          id: 'task-2',
          kind: 'message',
          contextId: 'chat-agent2',
          status: {
            state: 'working',
            timestamp: new Date(Date.now() - 1000 * 30).toISOString(),
          },
          artifacts: [],
          history: [],
        },
      ]);
      setError(null);
    } catch (err) {
      setError('Failed to fetch tasks');
      console.error('Error fetching tasks:', err);
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
    const now = new Date();
    const diff = now.getTime() - date.getTime();
    
    if (diff < 1000 * 60) {
      return 'Just now';
    } else if (diff < 1000 * 60 * 60) {
      return `${Math.floor(diff / (1000 * 60))} minutes ago`;
    } else if (diff < 1000 * 60 * 60 * 24) {
      return `${Math.floor(diff / (1000 * 60 * 60))} hours ago`;
    } else {
      return date.toLocaleDateString();
    }
  };

  if (error) {
    return (
      <div className="bg-white rounded-lg shadow p-6">
        <div className="text-center">
          <XCircle className="h-12 w-12 text-red-500 mx-auto mb-4" />
          <p className="text-red-600">{error}</p>
          <button
            onClick={fetchTasks}
            className="mt-4 px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="bg-white rounded-lg shadow">
      <div className="flex items-center justify-between p-6 border-b">
        <div className="flex items-center space-x-2">
          <Activity className="h-5 w-5 text-blue-600" />
          <h2 className="text-lg font-semibold text-gray-900">Task Monitor</h2>
        </div>
        <button
          onClick={fetchTasks}
          className="text-sm text-blue-600 hover:text-blue-800 font-medium"
        >
          Refresh
        </button>
      </div>

      <div className="p-6">
        {loading ? (
          <div className="flex items-center justify-center py-8">
            <Loader2 className="h-8 w-8 text-blue-600 animate-spin" />
            <span className="ml-2 text-gray-600">Loading tasks...</span>
          </div>
        ) : (
          <div className="space-y-4">
            {tasks.map((task) => (
              <div
                key={task.id}
                className="border border-gray-200 rounded-lg p-4 hover:shadow-md transition-shadow"
              >
                <div className="flex items-start justify-between">
                  <div className="flex items-start space-x-3">
                    {getStatusIcon(task.status.state)}
                    <div className="flex-1">
                      <div className="flex items-center space-x-2 mb-1">
                        <h3 className="font-medium text-gray-900">Task {task.id}</h3>
                        <span
                          className={`inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium border ${getStatusColor(
                            task.status.state
                          )}`}
                        >
                          {task.status.state}
                        </span>
                      </div>
                      
                      <div className="text-sm text-gray-600 space-y-1">
                        <p>Type: {task.kind}</p>
                        <p>Context: {task.contextId}</p>
                        {task.status.timestamp && (
                          <p>Last Updated: {formatTimestamp(task.status.timestamp)}</p>
                        )}
                      </div>
                    </div>
                  </div>
                  
                  <button
                    className="text-sm text-blue-600 hover:text-blue-800 font-medium"
                  >
                    View Details
                  </button>
                </div>
                
                {/* Display artifacts if any */}
                {task.artifacts.length > 0 && (
                  <div className="mt-4 pt-4 border-t border-gray-100">
                    <div className="flex items-center space-x-2 mb-3">
                      <FileText className="h-4 w-4 text-gray-500" />
                      <h4 className="text-sm font-medium text-gray-700">
                        Artifacts ({task.artifacts.length})
                      </h4>
                    </div>
                    <div className="space-y-3">
                      {task.artifacts.map((artifact) => (
                        <ArtifactRenderer
                          key={artifact.artifactId}
                          artifact={artifact}
                          className="bg-gray-50"
                        />
                      ))}
                    </div>
                  </div>
                )}
                
                {task.history.length > 0 && (
                  <div className="mt-4 pt-4 border-t border-gray-100">
                    <h4 className="text-sm font-medium text-gray-700 mb-2">History</h4>
                    <div className="text-sm text-gray-600">
                      {task.history.length} message(s)
                    </div>
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};

export default TaskMonitor;