import { useEffect, useState, useMemo } from 'react'
import { useSearchParams } from 'react-router-dom'
import { Chat, useAgent, useAgentDefinitions, useChatMessages } from '@distri/react'
import { Loader2 } from 'lucide-react'
import { v4 as uuidv4 } from 'uuid'

export default function ChatPage() {
  const [searchParams, setSearchParams] = useSearchParams()
  const { agents, loading: agentsLoading } = useAgentDefinitions()

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
          <select
            className="min-w-[200px] rounded border border-slate-700 bg-slate-900 px-2 py-1 text-sm text-slate-100 focus:outline-none focus:ring-1 focus:ring-slate-500"
            value={selectedAgentId || ''}
            onChange={(e) => handleAgentChange(e.target.value)}
            disabled={agentsLoading}
          >
            <option value="">Select agent</option>
            {agents.map((a) => (
              <option key={a.id || a.name} value={a.id || a.name}>
                {a.name || a.id}
              </option>
            ))}
          </select>
        </div>
      </header>

      {(agentLoading == true || messagesLoading == true) && <Loader2 className="animate-spin h-5 w-5" />}
      {agentLoading != true && agent && <div className="flex-1 min-h-0 overflow-hidden relative">
        <Chat
          key={threadId}
          agent={agent}
          threadId={threadId}
          initialMessages={messages}
          theme="dark"
        />
      </div>}
    </div>
  )
}
