import React, { useState, useRef, useEffect } from 'react';
import { Send, Loader2, User, Bot, Check, X, AlertCircle, Wrench, Brain } from 'lucide-react';

interface Agent {
  id: string;
  name: string;
  description: string;
  status: 'online' | 'offline';
}

interface ChatProps {
  agent: Agent;
  threadId: string;
}

interface ToolCall {
  id: string;
  name: string;
  args: string;
  status: 'pending_approval' | 'waiting_approval' | 'approved' | 'rejected' | 'executing' | 'completed' | 'error';
  parentMessageId?: string;
  result?: string;
  error?: string;
}

interface Message {
  id: string;
  role: 'user' | 'agent' | 'thinking' | 'system';
  content: string;
  timestamp: Date;
  taskId?: string;
  toolCalls?: ToolCall[];
  isStreaming?: boolean;
  runId?: string;
  thinkingId?: string;
}

const Chat: React.FC<ChatProps> = ({ agent, threadId }) => {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [toolCalls, setToolCalls] = useState<Map<string, ToolCall>>(new Map());
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const eventSourceRef = useRef<EventSource | null>(null);

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  };

  useEffect(() => {
    scrollToBottom();
  }, [messages]);

  useEffect(() => {
    // Set up SSE connection for real-time events, scoped by threadId
    setupEventSource();
    setMessages([]); // Reset messages when thread changes
    return () => {
      if (eventSourceRef.current) {
        eventSourceRef.current.close();
      }
    };
  }, [agent.id, threadId]);

  const setupEventSource = () => {
    if (eventSourceRef.current) {
      eventSourceRef.current.close();
    }
    // Optionally, pass threadId as a query param if backend supports filtering
    const eventSource = new EventSource(`/api/v1/agents/${agent.id}/events?thread_id=${threadId}`);
    eventSourceRef.current = eventSource;
    eventSource.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        // Only handle events for this thread
        if (!data.thread_id || data.thread_id === threadId) {
          handleStreamEvent(data);
        }
      } catch (error) {
        console.error('Error parsing SSE data:', error);
      }
    };
    eventSource.onerror = () => {
      console.warn('SSE connection error, will retry...');
    };
  };

  const handleStreamEvent = (data: any) => {
    const { type, run_id, task_id } = data;

    switch (type) {
      case 'RUN_STARTED':
        handleRunStarted(data);
        break;
      case 'TEXT_MESSAGE_START':
        handleMessageStart(data);
        break;
      case 'TEXT_MESSAGE_CONTENT':
        handleMessageStream(data);
        break;
      case 'TEXT_MESSAGE_END':
        handleMessageEnd(data);
        break;
      case 'TOOL_CALL_START':
        handleToolCallStart(data);
        break;
      case 'TOOL_CALL_ARGS':
        handleToolCallArgs(data);
        break;
      case 'TOOL_CALL_END':
        handleToolCallEnd(data);
        break;
      case 'TOOL_CALL_RESULT':
        handleToolResult(data);
        break;
      case 'RUN_FINISHED':
        handleRunFinished(data);
        break;
      case 'RUN_ERROR':
        handleRunError(data);
        break;
      case 'STATE_SNAPSHOT':
        // Optionally handle state snapshot
        break;
      case 'STATE_DELTA':
        // Optionally handle state delta
        break;
      case 'MESSAGES_SNAPSHOT':
        // Optionally handle messages snapshot
        break;
      case 'CUSTOM':
        if (data.customType === 'THINKING_START') handleThinkingStart(data);
        else if (data.customType === 'THINKING_CONTENT') handleThinkingStream(data);
        else if (data.customType === 'THINKING_END') handleThinkingEnd(data);
        break;
      // No legacy/old event names
    }
  };

  const handleRunStarted = (data: any) => {
    setIsLoading(true);
  };

  const handleMessageStart = (data: any) => {
    const { run_id, message_id, role, task_id } = data;

    if (role === 'assistant') {
      // Start a new assistant message
      setMessages((prev: Message[]) => [
        ...prev,
        {
          id: message_id,
          role: 'agent',
          content: '',
          timestamp: new Date(),
          isStreaming: true,
          taskId: task_id,
          runId: run_id,
        }
      ]);
    }
  };

  const handleMessageStream = (data: any) => {
    const { delta, task_id, run_id, message_id } = data;

    setMessages((prev: Message[]) => {
      const lastMessage = prev[prev.length - 1];
      if (lastMessage && lastMessage.role === 'agent' && lastMessage.isStreaming &&
        (lastMessage.taskId === task_id || lastMessage.runId === run_id)) {
        return [
          ...prev.slice(0, -1),
          {
            ...lastMessage,
            content: lastMessage.content + delta,
          }
        ];
      }
      return prev;
    });
  };

  const handleMessageEnd = (data: any) => {
    const { task_id, run_id, message_id } = data;

    setMessages((prev: Message[]) => {
      return prev.map(msg => {
        if ((msg.taskId === task_id || msg.runId === run_id) && msg.isStreaming) {
          return { ...msg, isStreaming: false };
        }
        return msg;
      });
    });
  };

  const handleThinkingStart = (data: any) => {
    const { run_id, thinking_id, task_id } = data;
    // Only one thinking message per thinking_id/run_id/threadId
    setMessages((prev: Message[]) => {
      // Remove any existing thinking message for this thinking_id/run_id/threadId
      const filtered = prev.filter(
        msg => !(msg.role === 'thinking' && msg.thinkingId === thinking_id && msg.runId === run_id)
      );
      return [
        ...filtered,
        {
          id: thinking_id,
          role: 'thinking',
          content: '',
          timestamp: new Date(),
          isStreaming: true,
          taskId: task_id,
          runId: run_id,
          thinkingId: thinking_id,
        }
      ];
    });
  };

  const handleThinkingStream = (data: any) => {
    const { delta, thinking_id, run_id } = data;
    setMessages((prev: Message[]) => {
      return prev.map(msg => {
        if (msg.role === 'thinking' && msg.thinkingId === thinking_id && msg.runId === run_id) {
          return {
            ...msg,
            content: msg.content + delta,
          };
        }
        return msg;
      });
    });
  };

  const handleThinkingEnd = (data: any) => {
    const { thinking_id, run_id } = data;
    setMessages((prev: Message[]) => {
      // Remove the thinking message for this thinking_id/run_id/threadId
      return prev.filter(
        msg => !(msg.role === 'thinking' && msg.thinkingId === thinking_id && msg.runId === run_id)
      );
    });
  };

  const handleRunFinished = (data: any) => {
    setIsLoading(false);

    // Mark all streaming messages as completed
    setMessages((prev: Message[]) => {
      return prev.map(msg => ({ ...msg, isStreaming: false }));
    });
  };

  const handleRunError = (data: any) => {
    setIsLoading(false);
    const { error, task_id, run_id } = data;

    // Add error message
    setMessages((prev: Message[]) => [
      ...prev,
      {
        id: `error-${Date.now()}`,
        role: 'system',
        content: `Error: ${error}`,
        timestamp: new Date(),
        isStreaming: false,
        taskId: task_id,
        runId: run_id,
      }
    ]);
  };

  const handleToolCallStart = (data: any) => {
    const toolCall: ToolCall = {
      id: data.tool_call_id,
      name: data.tool_name,
      args: '',
      status: 'pending_approval',
      parentMessageId: data.parent_message_id,
    };

    setToolCalls(prev => new Map(prev.set(data.tool_call_id, toolCall)));
  };

  const handleToolCallArgs = (data: any) => {
    setToolCalls((prev: Map<string, ToolCall>) => {
      const newMap = new Map(prev);
      const existing = newMap.get(data.tool_call_id);
      if (existing) {
        newMap.set(data.tool_call_id, {
          ...existing,
          args: existing.args + (data.args_delta || ''),
        });
      }
      return newMap;
    });
  };

  const handleToolCallEnd = (data: any) => {
    setToolCalls((prev: Map<string, ToolCall>) => {
      const newMap = new Map(prev);
      const existing = newMap.get(data.tool_call_id);
      if (existing) {
        newMap.set(data.tool_call_id, {
          ...existing,
          status: 'waiting_approval',
        });
      }
      return newMap;
    });
  };

  const handleToolCallApproved = (toolCallId: string) => {
    setToolCalls((prev: Map<string, ToolCall>) => {
      const newMap = new Map(prev);
      const existing = newMap.get(toolCallId);
      if (existing) {
        newMap.set(toolCallId, {
          ...existing,
          status: 'executing',
        });
      }
      return newMap;
    });
  };

  const handleToolCallRejected = (toolCallId: string) => {
    setToolCalls((prev: Map<string, ToolCall>) => {
      const newMap = new Map(prev);
      const existing = newMap.get(toolCallId);
      if (existing) {
        newMap.set(toolCallId, {
          ...existing,
          status: 'rejected',
        });
      }
      return newMap;
    });
  };

  const handleToolResult = (data: any) => {
    const { tool_call_id, result } = data;
    setToolCalls((prev: Map<string, ToolCall>) => {
      const newMap = new Map(prev);
      const existing = newMap.get(tool_call_id);
      if (existing) {
        newMap.set(tool_call_id, {
          ...existing,
          status: 'completed',
          result: result,
        });
      }
      return newMap;
    });
  };

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
      // Send message using streaming A2A protocol
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
              acceptedOutputModes: ['text/plain'],
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

      // Create initial streaming agent message
      const agentMessage: Message = {
        id: `${Date.now()}-agent`,
        role: 'agent',
        content: '',
        timestamp: new Date(),
        taskId: task.id,
        isStreaming: true,
      };

      setMessages(prev => [...prev, agentMessage]);

    } catch (error) {
      console.error('Failed to send message:', error);
      setIsLoading(false);
      const errorMessage: Message = {
        id: `${Date.now()}-error`,
        role: 'agent',
        content: `Error: ${error instanceof Error ? error.message : 'Failed to send message'}`,
        timestamp: new Date(),
      };
      setMessages(prev => [...prev, errorMessage]);
    }
  };

  const approveToolCall = async (toolCallId: string) => {
    try {
      await fetch(`/api/v1/tool-calls/${toolCallId}/approve`, {
        method: 'POST',
      });
    } catch (error) {
      console.error('Failed to approve tool call:', error);
    }
  };

  const rejectToolCall = async (toolCallId: string) => {
    try {
      await fetch(`/api/v1/tool-calls/${toolCallId}/reject`, {
        method: 'POST',
      });
    } catch (error) {
      console.error('Failed to reject tool call:', error);
    }
  };

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  };

  const renderToolCall = (toolCall: ToolCall) => {
    const getStatusColor = () => {
      switch (toolCall.status) {
        case 'waiting_approval':
          return 'bg-yellow-50 border-yellow-200';
        case 'approved':
        case 'executing':
          return 'bg-blue-50 border-blue-200';
        case 'completed':
          return 'bg-green-50 border-green-200';
        case 'rejected':
        case 'error':
          return 'bg-red-50 border-red-200';
        default:
          return 'bg-gray-50 border-gray-200';
      }
    };

    const getStatusIcon = () => {
      switch (toolCall.status) {
        case 'waiting_approval':
          return <AlertCircle className="h-4 w-4 text-yellow-600" />;
        case 'approved':
        case 'completed':
          return <Check className="h-4 w-4 text-green-600" />;
        case 'executing':
          return <Loader2 className="h-4 w-4 text-blue-600 animate-spin" />;
        case 'rejected':
        case 'error':
          return <X className="h-4 w-4 text-red-600" />;
        default:
          return <Wrench className="h-4 w-4 text-gray-600" />;
      }
    };

    return (
      <div key={toolCall.id} className={`border rounded-lg p-3 mb-2 ${getStatusColor()}`}>
        <div className="flex items-center justify-between mb-2">
          <div className="flex items-center space-x-2">
            {getStatusIcon()}
            <span className="font-medium text-sm">{toolCall.name}</span>
            <span className="text-xs text-gray-500 capitalize">{toolCall.status.replace('_', ' ')}</span>
          </div>
          {toolCall.status === 'waiting_approval' && (
            <div className="flex space-x-2">
              <button
                onClick={() => approveToolCall(toolCall.id)}
                className="px-2 py-1 bg-green-600 text-white text-xs rounded hover:bg-green-700 flex items-center space-x-1"
              >
                <Check className="h-3 w-3" />
                <span>Approve</span>
              </button>
              <button
                onClick={() => rejectToolCall(toolCall.id)}
                className="px-2 py-1 bg-red-600 text-white text-xs rounded hover:bg-red-700 flex items-center space-x-1"
              >
                <X className="h-3 w-3" />
                <span>Reject</span>
              </button>
            </div>
          )}
        </div>
        {toolCall.args && (
          <div className="bg-white bg-opacity-50 rounded p-2 text-xs">
            <strong>Arguments:</strong>
            <pre className="mt-1 whitespace-pre-wrap">{toolCall.args}</pre>
          </div>
        )}
        {toolCall.result && (
          <div className="bg-white bg-opacity-50 rounded p-2 text-xs mt-2">
            <strong>Result:</strong>
            <pre className="mt-1 whitespace-pre-wrap">{toolCall.result}</pre>
          </div>
        )}
        {toolCall.error && (
          <div className="bg-red-100 rounded p-2 text-xs mt-2">
            <strong>Error:</strong>
            <pre className="mt-1 whitespace-pre-wrap text-red-800">{toolCall.error}</pre>
          </div>
        )}
      </div>
    );
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
        <div className="flex items-center space-x-2">
          <div className={`w-2 h-2 rounded-full ${agent.status === 'online' ? 'bg-green-400' : 'bg-gray-400'}`} />
          <span className="text-xs text-gray-500">Streaming Mode</span>
        </div>
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        {messages.length === 0 && (
          <div className="text-center py-8">
            <Bot className="h-12 w-12 text-gray-400 mx-auto mb-4" />
            <p className="text-gray-500">Start a conversation with {agent.name}</p>
            <p className="text-xs text-gray-400 mt-2">Tool calls will require your approval</p>
          </div>
        )}

        {messages.map((message) => (
          <div key={message.id}>
            {/* Thinking messages - special styling */}
            {message.role === 'thinking' && (
              <div className="flex justify-start">
                <div className="max-w-[70%] rounded-lg px-4 py-2 bg-purple-50 border border-purple-200 text-purple-900">
                  <div className="flex items-start space-x-2">
                    <Brain className="h-4 w-4 mt-0.5 flex-shrink-0 text-purple-600" />
                    <div className="flex-1">
                      <div className="text-xs font-medium text-purple-600 mb-1">Agent Thinking</div>
                      <div className="text-sm font-mono whitespace-pre-wrap">
                        {message.content}
                        {message.isStreaming && (
                          <span className="inline-block w-2 h-4 bg-purple-600 ml-1 animate-pulse"></span>
                        )}
                      </div>
                      <p className="text-xs mt-1 text-purple-500">
                        {message.timestamp.toLocaleTimeString()}
                      </p>
                    </div>
                  </div>
                </div>
              </div>
            )}

            {/* Regular messages */}
            {message.role !== 'thinking' && (
              <div className={`flex ${message.role === 'user' ? 'justify-end' : 'justify-start'}`}>
                <div className={`max-w-[70%] rounded-lg px-4 py-2 ${message.role === 'user'
                  ? 'bg-blue-600 text-white'
                  : message.role === 'system'
                    ? 'bg-red-50 border border-red-200 text-red-900'
                    : 'bg-gray-100 text-gray-900'
                  }`}>
                  <div className="flex items-start space-x-2">
                    {message.role === 'agent' && (
                      <Bot className="h-4 w-4 mt-0.5 flex-shrink-0" />
                    )}
                    {message.role === 'user' && (
                      <User className="h-4 w-4 mt-0.5 flex-shrink-0" />
                    )}
                    {message.role === 'system' && (
                      <AlertCircle className="h-4 w-4 mt-0.5 flex-shrink-0 text-red-600" />
                    )}
                    <div className="flex-1">
                      <p className="whitespace-pre-wrap">
                        {message.content}
                        {message.isStreaming && (
                          <span className="inline-block w-2 h-4 bg-current ml-1 animate-pulse"></span>
                        )}
                      </p>
                      <p className={`text-xs mt-1 ${message.role === 'user' ? 'text-blue-200' :
                        message.role === 'system' ? 'text-red-500' : 'text-gray-500'
                        }`}>
                        {message.timestamp.toLocaleTimeString()}
                      </p>
                    </div>
                  </div>
                </div>
              </div>
            )}

            {/* Tool calls for this message */}
            {message.role === 'agent' && (
              <div className="mt-2 ml-8">
                {Array.from(toolCalls.values())
                  .filter(tc => tc.parentMessageId === message.id)
                  .map(renderToolCall)}
              </div>
            )}
          </div>
        ))}

        {/* Show pending tool calls that aren't attached to a specific message */}
        {Array.from(toolCalls.values())
          .filter(tc => !tc.parentMessageId && ['waiting_approval', 'executing'].includes(tc.status))
          .length > 0 && (
            <div className="border-t pt-4">
              <h4 className="text-sm font-medium text-gray-700 mb-2">Pending Tool Calls</h4>
              {Array.from(toolCalls.values())
                .filter(tc => !tc.parentMessageId && ['waiting_approval', 'executing'].includes(tc.status))
                .map(renderToolCall)}
            </div>
          )}

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