import React, { useState, useRef, useEffect } from 'react';
import { Send, Loader2, User, Bot } from 'lucide-react';
import { Message, TaskStatusUpdateEvent, TextPart } from '@a2a-js/sdk';
import MessageRenderer from './MessageRenderer';

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
        setMessages(threadMessages);
      } else {
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
      kind: 'message',
      messageId: Date.now().toString(),
      role: 'user',
      parts: [{ kind: 'text', text: input.trim() }],
      contextId: thread.id,
      taskId: undefined,
      referenceTaskIds: [],
      extensions: [],
      metadata: undefined,
    };

    setMessages((prev: Message[]) => [...prev, userMessage]);
    setInput('');
    setIsLoading(true);

    console.log("messages", messages)
    try {
      // Send message using A2A protocol with message/send_streaming method and thread.id as contextId
      const response = await fetch(`${apiUrl}/api/v1/agents/${agent.id}`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          jsonrpc: '2.0',
          method: 'message/stream',
          params: {
            message: userMessage,
            configuration: {
              acceptedOutputModes: ['text/plain'],
              blocking: false, // Use non-blocking for streaming
            },
          },
          id: userMessage.messageId,
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
            if (!result) continue;

            // Handle A2A streaming responses

            let message = undefined;
            if (result.kind === 'message') {
              message = (result as Message);
            } else if (result.kind === 'status-update') {
              message = (result as TaskStatusUpdateEvent).status.message as Message;
            }

            if (!message) continue;
            setMessages((prev: Message[]) => {
              if (prev.find(msg => msg.messageId === message.messageId)) {
                return prev.map(msg => {
                  if (msg.messageId === message.messageId) {
                    return {
                      ...msg,
                      parts: [...msg.parts, ...message.parts],
                    };
                  }
                  return msg;
                });
              } else {
                return [...prev, message];
              }
            });
            if (result.final) {
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
        kind: 'message',
        messageId: `${Date.now()}-error`,
        role: 'agent',
        parts: [{ kind: 'text', text: `Error: ${error instanceof Error ? error.message : 'Failed to send message'}` }],
        contextId: thread.id,
        taskId: undefined,
        referenceTaskIds: [],
        extensions: [],
        metadata: undefined,
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
    <div className="bg-white rounded-lg shadow h-full flex flex-col">
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

        {messages.filter(message => message.parts.length > 0).map((message) => {
          // Determine if this is an error message (starts with 'Error:')
          const isError = message.parts[0].kind === 'text' && message.parts[0].text.startsWith('Error:');
          return (
            <div
              key={message.messageId}
              className={`flex ${message.role === 'user' ? 'justify-end' : 'justify-start'}`}
            >
              <div
                className={`max-w-[70%] rounded-lg px-4 py-2 ${message.role === 'user'
                  ? 'bg-blue-600 text-white'
                  : isError
                    ? 'bg-red-100 text-red-800 border border-red-300'
                    : 'bg-gray-100 text-gray-900'
                  }`}
              >
                <div className="flex items-start space-x-2">
                  {message.role === 'agent' && (
                    <Bot className={`h-4 w-4 mt-0.5 flex-shrink-0 ${isError ? 'text-red-600' : ''}`}
                    />
                  )}
                  {message.role === 'user' && (
                    <User className="h-4 w-4 mt-0.5 flex-shrink-0" />
                  )}
                  <div className="flex-1">
                    <MessageRenderer
                      content={message.parts.map((part) => {
                        if (part.kind === 'text') {
                          return part.text;
                        }
                        return '';
                      }).join('')}
                      className={isError ? 'font-semibold' : ''}
                    />
                    <p className={`text-xs mt-1 ${message.role === 'user'
                      ? 'text-blue-200'
                      : isError
                        ? 'text-red-600'
                        : 'text-gray-500'
                      }`}>
                      {message.metadata?.timestamp ? new Date(message.metadata.timestamp as string).toLocaleTimeString() : ''}
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