"use client"

import { useState, useEffect, useRef } from 'react'
import { useChat } from 'ai/react'
import { Send, Bot, User, Settings, Sparkles, Loader2, AlertCircle } from 'lucide-react'
import ReactMarkdown from 'react-markdown'

interface Agent {
  id: string
  name: string
  description: string
  capabilities: Record<string, unknown>
}

export function ChatInterface() {
  const [agents, setAgents] = useState<Agent[]>([])
  const [selectedAgent, setSelectedAgent] = useState<string>('')
  const [useDistriAgent, setUseDistriAgent] = useState(false)
  const [isLoadingAgents, setIsLoadingAgents] = useState(false)
  const messagesEndRef = useRef<HTMLDivElement>(null)

  const { messages, input, handleInputChange, handleSubmit, isLoading, error } = useChat({
    api: '/api/chat',
    body: {
      useDistriAgent,
      agentId: selectedAgent,
    },
    onError: (error) => {
      console.error('Chat error:', error)
    },
  })

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }

  useEffect(() => {
    scrollToBottom()
  }, [messages])

  const loadAgents = async () => {
    setIsLoadingAgents(true)
    try {
      const response = await fetch('/api/chat')
      const data = await response.json()
      setAgents(data.agents || [])
    } catch (error) {
      console.error('Failed to load agents:', error)
    } finally {
      setIsLoadingAgents(false)
    }
  }

  useEffect(() => {
    loadAgents()
  }, [])

  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!input.trim()) return
    handleSubmit(e)
  }

  return (
    <div className="flex flex-col h-screen bg-gradient-to-br from-background via-background to-muted/20">
      {/* Header */}
      <div className="border-b bg-card/80 backdrop-blur-sm sticky top-0 z-10">
        <div className="max-w-4xl mx-auto px-4 py-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center space-x-3">
              <div className="flex items-center space-x-2">
                <div className="p-2 rounded-lg bg-primary/10">
                  <Sparkles className="h-5 w-5 text-primary" />
                </div>
                <div>
                  <h1 className="text-xl font-semibold text-foreground">LangDB Assistant</h1>
                  <p className="text-sm text-muted-foreground">
                    Powered by Distri & Vercel AI SDK
                  </p>
                </div>
              </div>
            </div>

            {/* Agent Toggle */}
            <div className="flex items-center space-x-4">
              <div className="flex items-center space-x-2">
                <Settings className="h-4 w-4 text-muted-foreground" />
                <label className="text-sm font-medium">Agent Mode</label>
                <button
                  onClick={() => setUseDistriAgent(!useDistriAgent)}
                  className={`
                    relative inline-flex h-6 w-11 items-center rounded-full transition-colors
                    ${useDistriAgent ? 'bg-primary' : 'bg-muted'}
                  `}
                >
                  <span
                    className={`
                      inline-block h-4 w-4 transform rounded-full bg-white transition-transform
                      ${useDistriAgent ? 'translate-x-6' : 'translate-x-1'}
                    `}
                  />
                </button>
              </div>

              {useDistriAgent && (
                <select
                  value={selectedAgent}
                  onChange={(e) => setSelectedAgent(e.target.value)}
                  className="text-sm border rounded-md px-3 py-1 bg-background"
                  disabled={isLoadingAgents}
                >
                  <option value="">Select Agent</option>
                  {agents.map((agent) => (
                    <option key={agent.id} value={agent.id}>
                      {agent.name}
                    </option>
                  ))}
                </select>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-y-auto">
        <div className="max-w-4xl mx-auto px-4 py-6">
          {messages.length === 0 ? (
            <div className="text-center py-12">
              <div className="inline-flex items-center justify-center w-16 h-16 rounded-full bg-primary/10 mb-4">
                <Bot className="w-8 h-8 text-primary" />
              </div>
              <h2 className="text-2xl font-semibold text-foreground mb-2">
                Welcome to LangDB Assistant
              </h2>
              <p className="text-muted-foreground max-w-md mx-auto">
                Start a conversation with our AI assistant. Enable Agent Mode to connect with specialized Distri agents for specific tasks.
              </p>

              {useDistriAgent && agents.length > 0 && (
                <div className="mt-6 p-4 bg-card rounded-lg border max-w-sm mx-auto">
                  <h3 className="font-medium text-foreground mb-2">Available Agents</h3>
                  <div className="space-y-2">
                    {agents.slice(0, 3).map((agent) => (
                      <div
                        key={agent.id}
                        className="text-sm text-muted-foreground cursor-pointer hover:text-foreground"
                        onClick={() => setSelectedAgent(agent.id)}
                      >
                        • {agent.name}
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          ) : (
            <div className="space-y-6">
              {messages.map((message) => (
                <div
                  key={message.id}
                  className={`flex items-start space-x-3 message-bubble ${
                    message.role === 'user' ? 'flex-row-reverse space-x-reverse' : ''
                  }`}
                >
                  <div
                    className={`
                      flex-shrink-0 w-8 h-8 rounded-full flex items-center justify-center
                      ${
                        message.role === 'user'
                          ? 'bg-primary text-primary-foreground'
                          : 'bg-muted text-muted-foreground'
                      }
                    `}
                  >
                    {message.role === 'user' ? (
                      <User className="w-4 h-4" />
                    ) : (
                      <Bot className="w-4 h-4" />
                    )}
                  </div>
                  <div
                    className={`
                      flex-1 max-w-3xl
                      ${message.role === 'user' ? 'text-right' : ''}
                    `}
                  >
                    <div
                      className={`
                        inline-block p-4 rounded-2xl
                        ${
                          message.role === 'user'
                            ? 'bg-primary text-primary-foreground'
                            : 'bg-card border'
                        }
                      `}
                    >
                      {message.role === 'user' ? (
                        <p className="text-sm">{message.content}</p>
                      ) : (
                        <div className="prose prose-sm max-w-none dark:prose-invert">
                          <ReactMarkdown>{message.content}</ReactMarkdown>
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              ))}

              {isLoading && (
                <div className="flex items-start space-x-3">
                  <div className="flex-shrink-0 w-8 h-8 rounded-full bg-muted flex items-center justify-center">
                    <Bot className="w-4 h-4 text-muted-foreground" />
                  </div>
                  <div className="flex-1">
                    <div className="inline-block p-4 rounded-2xl bg-card border">
                      <div className="flex items-center space-x-2 text-muted-foreground">
                        <Loader2 className="w-4 h-4 animate-spin" />
                        <span className="text-sm typing-indicator">Thinking...</span>
                      </div>
                    </div>
                  </div>
                </div>
              )}

              {error && (
                <div className="flex items-start space-x-3">
                  <div className="flex-shrink-0 w-8 h-8 rounded-full bg-destructive/10 flex items-center justify-center">
                    <AlertCircle className="w-4 h-4 text-destructive" />
                  </div>
                  <div className="flex-1">
                    <div className="inline-block p-4 rounded-2xl bg-destructive/10 border border-destructive/20">
                      <p className="text-sm text-destructive">
                        Sorry, I encountered an error. Please try again.
                      </p>
                    </div>
                  </div>
                </div>
              )}
            </div>
          )}
          <div ref={messagesEndRef} />
        </div>
      </div>

      {/* Input */}
      <div className="border-t bg-card/80 backdrop-blur-sm">
        <div className="max-w-4xl mx-auto px-4 py-4">
          <form onSubmit={onSubmit} className="flex items-end space-x-3">
            <div className="flex-1">
              <textarea
                value={input}
                onChange={handleInputChange}
                placeholder={
                  useDistriAgent && selectedAgent
                    ? `Message ${agents.find(a => a.id === selectedAgent)?.name || 'agent'}...`
                    : "Type your message..."
                }
                className="
                  w-full px-4 py-3 rounded-2xl border bg-background
                  resize-none focus:outline-none focus:ring-2 focus:ring-primary/20
                  placeholder:text-muted-foreground
                "
                rows={1}
                style={{ minHeight: '52px', maxHeight: '120px' }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !e.shiftKey) {
                    e.preventDefault()
                    onSubmit(e)
                  }
                }}
                disabled={isLoading}
              />
            </div>
            <button
              type="submit"
              disabled={!input.trim() || isLoading}
              className="
                flex items-center justify-center w-12 h-12 rounded-full
                bg-primary text-primary-foreground
                hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed
                transition-colors
              "
            >
              {isLoading ? (
                <Loader2 className="w-5 h-5 animate-spin" />
              ) : (
                <Send className="w-5 h-5" />
              )}
            </button>
          </form>
        </div>
      </div>
    </div>
  )
}