import { useState, useEffect } from 'react'
import { useSearchParams } from 'react-router-dom'
import { v4 as uuidv4 } from 'uuid'
import { Plus, Play, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Header } from '@/components/ui/header'
import { useThreadContext } from '@/components/ThreadContext'
import { ReplayChat, SkeletonCard } from '@/components/ReplayChat'
import { Agent } from '@distri/core'

interface AgentChatViewProps {
  agent: Agent
  agentId: string
}

export const AgentChatView = ({ agent, agentId }: AgentChatViewProps) => {
  const [searchParams, setSearchParams] = useSearchParams()
  const { getLastThreadId, setLastThreadId } = useThreadContext()
  const [playMode, setPlayMode] = useState(false)
  const [threadId, setThreadId] = useState<string | null>()
  const [composerPrefill, setComposerPrefill] = useState<string | undefined>(undefined)

  // Read threadId from query string
  const threadIdFromQuery = searchParams.get('threadId')
  const prefillFromQuery = searchParams.get('prefill')

  // Create new thread if none exists
  useEffect(() => {
    if (agentId) {
      const lastThreadId = getLastThreadId(agentId)
      if (threadIdFromQuery) {
        setThreadId(threadIdFromQuery)
      } else if (lastThreadId) {
        setThreadId(lastThreadId)
      } else {
        setThreadId(uuidv4())
      }
    }
  }, [agentId, threadIdFromQuery, getLastThreadId])

  useEffect(() => {
    if (prefillFromQuery) {
      setComposerPrefill(prefillFromQuery)
      setSearchParams(prev => {
        const next = new URLSearchParams(prev)
        next.delete('prefill')
        return next
      })
    }
  }, [prefillFromQuery, setSearchParams])

  // Function to create new chat
  const createNewChat = () => {
    const newThreadId = uuidv4()
    setThreadId(newThreadId)
    if (agentId) setLastThreadId(agentId, newThreadId)
    // Update URL with new threadId
    setSearchParams(prev => {
      const newParams = new URLSearchParams(prev)
      newParams.set('threadId', newThreadId)
      return newParams
    })
  }

  // Function to toggle play mode
  const togglePlayMode = () => {
    setPlayMode(!playMode)
  }

  // Function to exit play mode
  const exitPlayMode = () => {
    setPlayMode(false)
  }

  return (
    <div className="flex flex-col h-screen relative w-full">
      <Header
        leftElement={
          <>
            <h1 className="text-2xl font-bold">{agent?.name}</h1>
          </>
        }
        rightElement={
          <div className="flex items-center gap-2">
            {playMode && (
              <Button
                variant="outline"
                size="icon"
                onClick={exitPlayMode}
                title="Exit play mode"
              >
                <X className="h-4 w-4" />
              </Button>
            )}

            <Button
              variant={playMode ? "default" : "outline"}
              size="icon"
              onClick={togglePlayMode}
              title={playMode ? "Exit play mode" : "Enter play mode"}
            >
              <Play className="h-4 w-4" />
            </Button>

            <Button
              variant="ghost"
              size="icon"
              onClick={createNewChat}
              className="flex items-center space-x-1"
            >
              <Plus className="h-4 w-4" />
            </Button>
          </div>
        }
      />

      <div className="flex-1 flex overflow-hidden min-h-0 gap-2">
        {threadId && agent ? (
          <div className="flex-1 overflow-hidden">
            <ReplayChat
              agent={agent}
              threadId={threadId}
              playMode={playMode ? "replay" : "chat"}
              initialInput={composerPrefill}
            />
          </div>
        ) : (
          <div className="flex-1 overflow-hidden">
            <SkeletonCard />
          </div>
        )}
      </div>
    </div>
  )
}
