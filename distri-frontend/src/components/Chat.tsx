import React, { useState, useRef, useEffect } from 'react';
import { Send, Loader2, User, Bot } from 'lucide-react';
import { v4 as uuidv4 } from 'uuid';

const apiUrl = 'http://localhost:8080';
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
      const response = await fetch(`${apiUrl}/api/v1/threads/${thread.id}/messages`);
      if (response.ok) {
        const threadMessages = await response.json();

        // Convert A2A messages to our Message format
        const convertedMessages: Message[] = threadMessages.map((msg: any, index: number) => ({
          id: msg.messageId || msg.message_id || `msg-${index}`,
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
      const response = await fetch(`${apiUrl}/api/v1/agents/${agent.id}`, {
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

      if (!response.body) throw new Error('No response body');

      // Set up streaming reader for SSE/JSON-RPC
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';
      let done = false;

      while (!done) {
        const { value, done: streamDone } = await reader.read();
        if (streamDone) break;
        buffer += decoder.decode(value, { stream: true });

        // Process all complete SSE events in the buffer
        let eventEnd;
        while ((eventEnd = buffer.indexOf('\n\n')) !== -1) {
          const eventStr = buffer.slice(0, eventEnd);
          buffer = buffer.slice(eventEnd + 2);

          // Extract all data: lines and join them
          const dataLines = eventStr
            .split('\n')
            .filter(line => line.startsWith('data:'))
            .map(line => line.slice(5).trim());
          if (dataLines.length === 0) continue;
          const jsonStr = dataLines.join('');
          if (!jsonStr) continue;

          try {
            const json = JSON.parse(jsonStr);
            if (json.error) {
              throw new Error(json.error.message);
            }
            const result = json.result;
            console.log(result);
            if (!result) continue;
            // Handle streaming updates
            if (result.status && result.status.message && result.status.message.role === 'agent' && result.status.message.parts) {
              const delta = result.status.message.parts.map((p: any) => p.text).join(' ');

              const messageId = result.status.message.messageId || result.status.message.message_id;
              const isPreviousMessage = messages.find(msg => msg.id === messageId);
              if (!isPreviousMessage) {
                const agentMessage: Message = {
                  id: messageId,
                  role: 'agent',
                  content: delta,
                  timestamp: new Date(),
                  taskId: messageId,
                };
                setMessages((prev: Message[]) => [...prev, agentMessage]);
              } else {
                setMessages((prev: Message[]) => {
                  return prev.map(msg => {
                    if (msg.id === messageId) {
                      return {
                        ...msg,
                        content: msg.content + delta,
                      };
                    }
                    return msg;
                  });
                });
              }
            }

            if (result.finalUpdate || result.final) {
              done = true;
              // Optionally update thread in parent component
              if (onThreadUpdate) {
                onThreadUpdate();
              }
              console.log(messages);
            }
          } catch (err) {

            done = true;
          }
        }
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

        {messages.filter(message => message.content.length > 0).map((message) => {
          // Determine if this is an error message (starts with 'Error:')
          const isError = message.content.startsWith('Error:');
          return (
            <div
              key={message.id}
              className={`flex ${message.role === 'user' ? 'justify-end' : 'justify-start'}`}
            >
              <div
                className={`max-w-[70%] rounded-lg px-4 py-2 ${message.role === 'user'
                  ? 'bg-blue-600 text-white'
                  : isError
                    ? 'bg-red-100 text-red-800 border border-red-300'
                    : message.type === 'thinking'
                      ? 'bg-yellow-50 text-yellow-800 border border-yellow-200'
                      : 'bg-gray-100 text-gray-900'
                  }`}
              >
                <div className="flex items-start space-x-2">
                  {message.role === 'agent' && (
                    <Bot className={`h-4 w-4 mt-0.5 flex-shrink-0 ${message.type === 'thinking' ? 'text-yellow-600' : isError ? 'text-red-600' : ''}`}
                    />
                  )}
                  {message.role === 'user' && (
                    <User className="h-4 w-4 mt-0.5 flex-shrink-0" />
                  )}
                  <div className="flex-1">
                    <p className={`whitespace-pre-wrap ${message.type === 'thinking' ? 'italic' : ''} ${isError ? 'font-semibold' : ''}`}>
                      {message.content}
                    </p>
                    <p className={`text-xs mt-1 ${message.role === 'user'
                      ? 'text-blue-200'
                      : message.type === 'thinking'
                        ? 'text-yellow-600'
                        : isError
                          ? 'text-red-600'
                          : 'text-gray-500'
                      }`}>
                      {message.timestamp.toLocaleTimeString()}
                    </p>
                  </div>
                </div>
              </div>
            </div>
          );
        })}

        {isLoading && (
          <div className="flex justify-start">
            <Loader2 className="h-4 w-4 animate-spin" />
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