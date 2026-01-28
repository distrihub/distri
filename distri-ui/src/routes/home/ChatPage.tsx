import { useEffect, useState, useMemo, useRef } from 'react'
import { useSearchParams } from 'react-router-dom'
import { Chat, useAgent, useAgentsByUsage, useChatMessages } from '@distri/react'
import { ChevronDown, Loader2, Search, X } from 'lucide-react'
import { v4 as uuidv4 } from 'uuid'
import type { AgentUsageInfo } from '@distri/core'

function AgentSearchDropdown({
  agents,
  loading,
  selectedAgentId,
  onSelect,
  search,
  onSearchChange,
}: {
  agents: AgentUsageInfo[]
  loading: boolean
  selectedAgentId: string | undefined
  onSelect: (agentId: string) => void
  search: string
  onSearchChange: (search: string) => void
}) {
  const [open, setOpen] = useState(false)
  const containerRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  const selectedAgent = agents.find((a) => a.agent_id === selectedAgentId)
  const displayName = selectedAgent?.agent_name || selectedAgent?.agent_id || 'Select agent'

  const filteredAgents = useMemo(() => {
    if (!search) return agents
    const lower = search.toLowerCase()
    return agents.filter(
      (a) =>
        a.agent_name.toLowerCase().includes(lower) ||
        a.agent_id.toLowerCase().includes(lower)
    )
  }, [agents, search])

  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false)
      }
    }
    if (open) {
      document.addEventListener('mousedown', handleClickOutside)
      return () => document.removeEventListener('mousedown', handleClickOutside)
    }
  }, [open])

  useEffect(() => {
    if (open) {
      setTimeout(() => inputRef.current?.focus(), 0)
    } else {
      onSearchChange('')
    }
  }, [open, onSearchChange])

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        disabled={loading}
        className="flex min-w-[200px] items-center justify-between gap-2 rounded border border-slate-700 bg-slate-900 px-2 py-1 text-sm text-slate-100 focus:outline-none focus:ring-1 focus:ring-slate-500"
      >
        <span className="truncate">{loading ? 'Loading...' : displayName}</span>
        <ChevronDown className="h-3.5 w-3.5 shrink-0 text-slate-400" />
      </button>

      {open && (
        <div className="absolute left-0 top-full z-50 mt-1 w-72 rounded-md border border-slate-700 bg-slate-900 shadow-xl">
          <div className="flex items-center border-b border-slate-700 px-2">
            <Search className="h-3.5 w-3.5 shrink-0 text-slate-500" />
            <input
              ref={inputRef}
              type="text"
              value={search}
              onChange={(e) => onSearchChange(e.target.value)}
              placeholder="Search agents..."
              className="h-9 w-full bg-transparent px-2 text-sm text-slate-100 placeholder-slate-500 focus:outline-none"
            />
            {search && (
              <button
                type="button"
                onClick={() => onSearchChange('')}
                className="text-slate-500 hover:text-slate-300"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            )}
          </div>
          <div className="max-h-64 overflow-y-auto py-1">
            {filteredAgents.length === 0 ? (
              <div className="px-3 py-2 text-sm text-slate-500">No agents found</div>
            ) : (
              filteredAgents.map((a) => (
                <button
                  key={a.agent_id}
                  type="button"
                  onClick={() => {
                    onSelect(a.agent_id)
                    setOpen(false)
                  }}
                  className={`flex w-full items-center justify-between px-3 py-2 text-left text-sm transition hover:bg-slate-800 ${
                    a.agent_id === selectedAgentId
                      ? 'bg-slate-800 text-slate-100'
                      : 'text-slate-300'
                  }`}
                >
                  <span className="truncate">{a.agent_name || a.agent_id}</span>
                  {a.thread_count > 0 && (
                    <span className="ml-2 shrink-0 text-xs text-slate-500">
                      {a.thread_count} threads
                    </span>
                  )}
                </button>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  )
}

export default function ChatPage() {
  const [searchParams, setSearchParams] = useSearchParams()
  const { agents, loading: agentsLoading, search, setSearch } = useAgentsByUsage()

  const agentIdParam = searchParams.get('id')
  const threadIdParam = searchParams.get('threadId')

  const [selectedAgentId, setSelectedAgentId] = useState<string | undefined>(agentIdParam || undefined)
  const { agent, loading: agentLoading } = useAgent({ agentIdOrDef: selectedAgentId || '' })
  const { messages, isLoading: messagesLoading } = useChatMessages({ agent: agent || undefined, threadId: threadIdParam || undefined })

  const threadId = useMemo(() => {
    if (threadIdParam) return threadIdParam
    return uuidv4()
  }, [threadIdParam])

  // Sync state with URL
  useEffect(() => {
    if (agentIdParam && agentIdParam !== selectedAgentId) {
      setSelectedAgentId(agentIdParam)
    }
  }, [agentIdParam])

  // Ensure threadId is in URL if missing
  useEffect(() => {
    if (!threadIdParam) {
      const newParams = new URLSearchParams(searchParams)
      newParams.set('threadId', threadId)
      setSearchParams(newParams, { replace: true })
    }
  }, [threadIdParam, threadId, searchParams, setSearchParams])

  const handleAgentChange = (newId: string) => {
    setSelectedAgentId(newId)
    const newParams = new URLSearchParams(searchParams)
    if (newId) {
      newParams.set('id', newId)
    } else {
      newParams.delete('id')
    }
    const newThreadId = uuidv4()
    newParams.set('threadId', newThreadId)
    setSearchParams(newParams)
  }

  return (
    <div className="flex h-full w-full flex-col bg-slate-950 text-slate-50">
      <header className="flex items-center gap-4 border-b border-slate-800 px-4 py-3 bg-slate-950">
        <div className="flex items-center gap-2">
          <label className="text-[11px] uppercase tracking-[0.2em] text-slate-500">
            Agent
          </label>
          <AgentSearchDropdown
            agents={agents}
            loading={agentsLoading}
            selectedAgentId={selectedAgentId}
            onSelect={handleAgentChange}
            search={search}
            onSearchChange={setSearch}
          />
        </div>
      </header>

      {(agentLoading == true || messagesLoading == true) && <Loader2 className="animate-spin h-5 w-5" />}
      {agentLoading != true && <div className="flex-1 min-h-0 overflow-hidden relative">
        <Chat
          key={threadId}
          threadId={threadId}
          initialMessages={messages}
          theme="dark"
        />
      </div>}
    </div>
  )
}
