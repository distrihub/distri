import React, { useState, useRef, useEffect } from 'react';
import { Send, Loader2, User, Bot } from 'lucide-react';

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

interface ChatProps {
  thread: Thread;
  agent: Agent;
  onThreadUpdate?: () => void;
}

interface Message {
  id: string;
  role: 'user' | 'agent';
  content: string;
  timestamp: Date;
  taskId?: string;
  type?: 'normal' | 'thinking';
}

const Chat: React.FC<ChatProps> = ({ thread, agent, onThreadUpdate }) => {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  };

  useEffect(() => {
    scrollToBottom();
  }, [messages]);

  // Load thread messages when thread changes
  useEffect(() => {
    if (thread) {
      loadThreadMessages();
    }
  }, [thread.id]);

  const loadThreadMessages = async () => {
    try {
      const response = await fetch(`/api/v1/threads/${thread.id}/messages`);
      if (response.ok) {
        const threadMessages = await response.json();
        
        // Convert A2A messages to our Message format
        const convertedMessages: Message[] = threadMessages.map((msg: any, index: number) => ({
          id: msg.messageId || `msg-${index}`,
          role: msg.role === 'user' ? 'user' : 'agent',
          content: msg.parts
            ?.filter((part: any) => part.kind === 'text')
            ?.map((part: any) => part.text)
            ?.join(' ') || '',
          timestamp: new Date(msg.metadata?.timestamp || Date.now()),
        }));
        
        setMessages(convertedMessages);
      } else {
        // If we can't load messages, start with empty state
        setMessages([]);
      }
    } catch (error) {
      console.error('Failed to load thread messages:', error);
      setMessages([]);
    }
  };

  const sendMessage = async () => {
    if (!input.trim() || isLoading) return;

    const userMessage: Message = {
      id: Date.now().toString(),
      role: 'user',
      content: input.trim(),
      timestamp: new Date(),
    };

    setMessages((prev: Message[]) => [...prev, userMessage]);
    setInput('');
    setIsLoading(true);

    try {
      // Send message using A2A protocol with message/send_streaming method and thread.id as contextId
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
              contextId: thread.id, // Use thread ID as context ID
            },
            configuration: {
              acceptedOutputModes: ['text/plain'],
              blocking: false, // Use non-blocking for streaming
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

      // Create initial empty agent message for streaming
      const agentMessage: Message = {
        id: `${Date.now()}-agent`,
        role: 'agent',
        content: '',
        timestamp: new Date(),
        taskId: task.id,
      };

      setMessages((prev: Message[]) => [...prev, agentMessage]);

      // Set up SSE listener for real-time updates
      setupSSEListener(task.id, agentMessage.id);

      // Update thread in parent component
      if (onThreadUpdate) {
        onThreadUpdate();
      }

    } catch (error) {
      console.error('Failed to send message:', error);
      const errorMessage: Message = {
        id: `${Date.now()}-error`,
        role: 'agent',
        content: `Error: ${error instanceof Error ? error.message : 'Failed to send message'}`,
        timestamp: new Date(),
      };
      setMessages((prev: Message[]) => [...prev, errorMessage]);
    } finally {
      setIsLoading(false);
    }
  };

  const setupSSEListener = (taskId: string, agentMessageId: string) => {
    // Listen to events filtered by thread ID for better performance
    const eventSource = new EventSource(`/api/v1/agents/${agent.id}/events?thread_id=${thread.id}`);

    let thinkingMessage: Message | null = null;

    eventSource.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);

        if (data.task_id === taskId && data.thread_id === thread.id) {
          if (data.type === 'run_started') {
            // Task has started
            console.log('Task started:', data);
          } else if (data.type === 'thinking_started') {
            // Agent is planning/thinking - show thinking indicator
            thinkingMessage = {
              id: `${Date.now()}-thinking`,
              role: 'agent',
              content: '🤔 Thinking...',
              timestamp: new Date(),
              taskId: taskId,
              type: 'thinking',
            };
            setMessages((prev: Message[]) => [...prev, thinkingMessage!]);
          } else if (data.type === 'thinking_finished') {
            // Remove thinking indicator
            if (thinkingMessage) {
              setMessages((prev: Message[]) => 
                prev.filter(msg => msg.id !== thinkingMessage!.id)
              );
              thinkingMessage = null;
            }
          } else if (data.type === 'text_delta') {
            // Update the agent message with streaming content
            setMessages((prev: Message[]) => {
              return prev.map(msg => {
                if (msg.id === agentMessageId) {
                  return {
                    ...msg,
                    content: msg.content + data.delta,
                  };
                }
                return msg;
              });
            });
          } else if (data.type === 'task_completed') {
            // Remove any remaining thinking message
            if (thinkingMessage) {
              setMessages((prev: Message[]) => 
                prev.filter(msg => msg.id !== thinkingMessage!.id)
              );
              thinkingMessage = null;
            }
            eventSource.close();
            // Update thread in parent component when task completes
            if (onThreadUpdate) {
              onThreadUpdate();
            }
          } else if (data.type === 'task_error') {
            // Remove any remaining thinking message
            if (thinkingMessage) {
              setMessages((prev: Message[]) => 
                prev.filter(msg => msg.id !== thinkingMessage!.id)
              );
              thinkingMessage = null;
            }
            
            // Update the agent message with error content
            setMessages((prev: Message[]) => {
              return prev.map(msg => {
                if (msg.id === agentMessageId) {
                  return {
                    ...msg,
                    content: `Error: ${data.error}`,
                  };
                }
                return msg;
              });
            });
            
            eventSource.close();
          }
        }
      } catch (error) {
        console.error('Error parsing SSE data:', error);
      }
    };

    eventSource.onerror = () => {
      if (thinkingMessage) {
        setMessages((prev: Message[]) => 
          prev.filter(msg => msg.id !== thinkingMessage!.id)
        );
      }
      eventSource.close();
    };

    // Clean up after 30 seconds
    setTimeout(() => {
      if (thinkingMessage) {
        setMessages((prev: Message[]) => 
          prev.filter(msg => msg.id !== thinkingMessage!.id)
        );
      }
      eventSource.close();
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
            <h3 className="font-medium text-gray-900">{thread.title}</h3>
            <p className="text-sm text-gray-500">with {agent.name}</p>
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
            <p className="text-gray-500">Continue your conversation with {agent.name}</p>
            <p className="text-sm text-gray-400 mt-1">
              Thread: "{thread.title}"
            </p>
          </div>
        )}

        {messages.map((message) => (
          <div
            key={message.id}
            className={`flex ${message.role === 'user' ? 'justify-end' : 'justify-start'}`}
          >
            <div
              className={`max-w-[70%] rounded-lg px-4 py-2 ${
                message.role === 'user'
                  ? 'bg-blue-600 text-white'
                  : message.type === 'thinking'
                  ? 'bg-yellow-50 text-yellow-800 border border-yellow-200'
                  : 'bg-gray-100 text-gray-900'
              }`}
            >
              <div className="flex items-start space-x-2">
                {message.role === 'agent' && (
                  <Bot className={`h-4 w-4 mt-0.5 flex-shrink-0 ${
                    message.type === 'thinking' ? 'text-yellow-600' : ''
                  }`} />
                )}
                {message.role === 'user' && (
                  <User className="h-4 w-4 mt-0.5 flex-shrink-0" />
                )}
                <div className="flex-1">
                  <p className={`whitespace-pre-wrap ${
                    message.type === 'thinking' ? 'italic' : ''
                  }`}>
                    {message.content}
                  </p>
                  <p className={`text-xs mt-1 ${
                    message.role === 'user' 
                      ? 'text-blue-200' 
                      : message.type === 'thinking'
                      ? 'text-yellow-600'
                      : 'text-gray-500'
                  }`}>
                    {message.timestamp.toLocaleTimeString()}
                  </p>
                </div>
              </div>
            </div>
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