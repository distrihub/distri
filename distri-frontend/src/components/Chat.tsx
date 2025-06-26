import React, { useState, useRef, useEffect } from 'react';
import { Send, Loader2, User, Bot } from 'lucide-react';
import ArtifactRenderer from './ArtifactRenderer';

interface Agent {
  id: string;
  name: string;
  description: string;
  status: 'online' | 'offline';
}

interface ChatProps {
  agent: Agent;
}

interface Message {
  id: string;
  role: 'user' | 'agent';
  content: string;
  timestamp: Date;
  taskId?: string;
}

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

const Chat: React.FC<ChatProps> = ({ agent }) => {
  const [messages, setMessages] = useState<Message[]>([]);
  const [artifacts, setArtifacts] = useState<{ [taskId: string]: Artifact[] }>({});
  const [input, setInput] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  };

  useEffect(() => {
    scrollToBottom();
  }, [messages, artifacts]);

  const sendMessage = async () => {
    if (!input.trim() || isLoading) return;

    const userMessage: Message = {
      id: Date.now().toString(),
      role: 'user',
      content: input.trim(),
      timestamp: new Date(),
    };

    setMessages(prev => [...prev, userMessage]);
    setInput('');
    setIsLoading(true);

    try {
      // Send message using A2A protocol with streaming
      const response = await fetch(`/api/v1/agents/${agent.id}`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          jsonrpc: '2.0',
          method: 'message/send_streaming',
          params: {
            message: {
              messageId: userMessage.id,
              role: 'user',
              parts: [
                {
                  kind: 'text',
                  text: userMessage.content,
                }
              ],
              contextId: `chat-${agent.id}`,
            },
            configuration: {
              acceptedOutputModes: ['text/plain', 'text/markdown'],
              blocking: false,
            },
          },
          id: userMessage.id,
        }),
      });

      const result = await response.json();

      if (result.error) {
        throw new Error(result.error.message);
      }

      const task = result.result;
      
      // Start with an empty agent message that will be streamed
      const agentMessage: Message = {
        id: `${Date.now()}-agent`,
        role: 'agent',
        content: '',
        timestamp: new Date(),
        taskId: task.id,
      };

      setMessages(prev => [...prev, agentMessage]);

      // Set up SSE listener for real-time updates
      setupSSEListener(task.id);

    } catch (error) {
      console.error('Failed to send message:', error);
      const errorMessage: Message = {
        id: `${Date.now()}-error`,
        role: 'agent',
        content: `Error: ${error instanceof Error ? error.message : 'Failed to send message'}`,
        timestamp: new Date(),
      };
      setMessages(prev => [...prev, errorMessage]);
    } finally {
      setIsLoading(false);
    }
  };

  const setupSSEListener = (taskId: string) => {
    const eventSource = new EventSource(`/api/v1/agents/${agent.id}/events`);

    eventSource.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);

        if (data.task_id === taskId || data.taskId === taskId) {
          if (data.type === 'text_delta') {
            // Update the last agent message with streaming content
            setMessages(prev => {
              const lastMessage = prev[prev.length - 1];
              if (lastMessage && lastMessage.role === 'agent' && lastMessage.taskId === taskId) {
                return [
                  ...prev.slice(0, -1),
                  {
                    ...lastMessage,
                    content: lastMessage.content + data.delta,
                  }
                ];
              }
              return prev;
            });
          } else if (data.kind === 'artifact-update') {
            // Handle artifact updates
            const artifact = data.artifact;
            setArtifacts(prev => {
              const taskArtifacts = prev[taskId] || [];
              const existingIndex = taskArtifacts.findIndex(a => a.artifactId === artifact.artifactId);
              
              if (existingIndex >= 0) {
                // Update existing artifact
                const updated = [...taskArtifacts];
                updated[existingIndex] = artifact;
                return { ...prev, [taskId]: updated };
              } else {
                // Add new artifact
                return { ...prev, [taskId]: [...taskArtifacts, artifact] };
              }
            });
          } else if (data.type === 'task_completed' || data.type === 'task_error') {
            eventSource.close();
            setIsLoading(false);
          }
        }
      } catch (error) {
        console.error('Error parsing SSE data:', error);
      }
    };

    eventSource.onerror = () => {
      eventSource.close();
      setIsLoading(false);
    };

    // Clean up after 30 seconds
    setTimeout(() => {
      eventSource.close();
      setIsLoading(false);
    }, 30000);
  };

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  };

  return (
    <div className="bg-white rounded-lg shadow h-[600px] flex flex-col">
      {/* Chat Header */}
      <div className="flex items-center justify-between p-4 border-b">
        <div className="flex items-center space-x-3">
          <div className="w-8 h-8 bg-blue-600 rounded-full flex items-center justify-center">
            <Bot className="h-4 w-4 text-white" />
          </div>
          <div>
            <h3 className="font-medium text-gray-900">{agent.name}</h3>
            <p className="text-sm text-gray-500">{agent.description}</p>
          </div>
        </div>
        <div className={`w-2 h-2 rounded-full ${agent.status === 'online' ? 'bg-green-400' : 'bg-gray-400'
          }`} />
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        {messages.length === 0 && (
          <div className="text-center py-8">
            <Bot className="h-12 w-12 text-gray-400 mx-auto mb-4" />
            <p className="text-gray-500">Start a conversation with {agent.name}</p>
          </div>
        )}

        {messages.map((message) => (
          <div key={message.id}>
            <div
              className={`flex ${message.role === 'user' ? 'justify-end' : 'justify-start'}`}
            >
              <div
                className={`max-w-[70%] rounded-lg px-4 py-2 ${message.role === 'user'
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
                    <p className="whitespace-pre-wrap">{message.content}</p>
                    <p className={`text-xs mt-1 ${message.role === 'user' ? 'text-blue-200' : 'text-gray-500'
                      }`}>
                      {message.timestamp.toLocaleTimeString()}
                    </p>
                  </div>
                </div>
              </div>
            </div>
            
            {/* Render artifacts for this message */}
            {message.taskId && artifacts[message.taskId] && artifacts[message.taskId].length > 0 && (
              <div className="mt-4 space-y-3">
                {artifacts[message.taskId].map((artifact) => (
                  <ArtifactRenderer
                    key={artifact.artifactId}
                    artifact={artifact}
                    className="max-w-[90%] mx-auto"
                  />
                ))}
              </div>
            )}
          </div>
        ))}

        {isLoading && (
          <div className="flex justify-start">
            <div className="bg-gray-100 rounded-lg px-4 py-2">
              <div className="flex items-center space-x-2">
                <Bot className="h-4 w-4" />
                <Loader2 className="h-4 w-4 animate-spin" />
                <span className="text-gray-600">Thinking...</span>
              </div>
            </div>
          </div>
        )}

        <div ref={messagesEndRef} />
      </div>

      {/* Input */}
      <div className="p-4 border-t">
        <div className="flex space-x-2">
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyPress={handleKeyPress}
            placeholder={`Message ${agent.name}...`}
            className="flex-1 border border-gray-300 rounded-lg px-3 py-2 resize-none focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
            rows={1}
            disabled={isLoading}
          />
          <button
            onClick={sendMessage}
            disabled={!input.trim() || isLoading}
            className="bg-blue-600 text-white rounded-lg px-4 py-2 hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed flex items-center space-x-1"
          >
            {isLoading ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Send className="h-4 w-4" />
            )}
          </button>
        </div>
      </div>
    </div>
  );
};

export default Chat;