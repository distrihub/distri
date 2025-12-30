import { useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Card, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { Badge } from '@/components/ui/badge'
import { ArrowRight, Bot, Workflow } from 'lucide-react'
import { useAgentDefinitions, ChatInput, useAgent, Chat } from '@distri/react'
import { AgentDefinition, DistriPart } from '@distri/core'
import { Avatar, AvatarFallback, AvatarImage } from '@/components/ui/avatar'
import { v4 as uuidv4 } from 'uuid'

type PackageFilterState =
  | { kind: 'all' }
  | { kind: 'default' }
  | { kind: 'local' }
  | { kind: 'plugin'; name: string }

interface AgentsHomeProps {
  showDesignerChat?: boolean
  enableNavigation?: boolean
  allowInlineCreator?: boolean
  onRequestCreateAgent?: () => void
  onSelectAgent?: (agent: AgentDefinition) => void
}

const AgentsHome = ({
  showDesignerChat: showDesignerChatProp = true,
  enableNavigation = true,
  allowInlineCreator = true,
  onRequestCreateAgent,
  onSelectAgent,
}: AgentsHomeProps) => {
  const navigate = useNavigate()
  const { agents, loading } = useAgentDefinitions()
  const [heroPrompt, setHeroPrompt] = useState('')
  const [showCreator, setShowCreator] = useState(false)
  const [packageFilter, setPackageFilter] = useState<PackageFilterState>({ kind: 'all' })
  const [showDesignerChat, setShowDesignerChat] = useState(showDesignerChatProp)
  const designerThreadIdRef = useRef<string | null>(null)
  useEffect(() => {
    setShowDesignerChat(showDesignerChatProp)
  }, [showDesignerChatProp])
  if (!designerThreadIdRef.current) {
    if (typeof window === 'undefined') {
      designerThreadIdRef.current = uuidv4()
    } else {
      const key = 'designer:threadId'
      designerThreadIdRef.current = window.localStorage.getItem(key) ?? uuidv4()
      window.localStorage.setItem(key, designerThreadIdRef.current)
    }
  }

  const packageSummary = useMemo(() => {
    const pluginNames = new Set<string>()
    let hasLocal = false
    let hasDefault = false

    agents.forEach((agent) => {
      const pkg = agent.package_name?.trim()
      if (pkg) {
        if (pkg === 'distri') {
          hasDefault = true
        } else {
          pluginNames.add(pkg)
        }
      } else {
        hasLocal = true
      }
    })

    return {
      pluginNames: Array.from(pluginNames).sort((a, b) => a.localeCompare(b)),
      hasLocal,
      hasDefault,
    }
  }, [agents])

  useEffect(() => {
    if (packageFilter.kind === 'all') {
      return
    }

    const stillExists = (() => {
      switch (packageFilter.kind) {
        case 'default':
          return agents.some((agent) => agent.package_name === 'distri')
        case 'local':
          return agents.some((agent) => !agent.package_name)
        case 'plugin':
          return agents.some((agent) => agent.package_name === packageFilter.name)
        default:
          return true
      }
    })()

    if (!stillExists) {
      setPackageFilter({ kind: 'all' })
    }
  }, [agents, packageFilter])

  const filteredAgents = useMemo(() => {
    switch (packageFilter.kind) {
      case 'all':
        return agents
      case 'default':
        return agents.filter((agent) => agent.package_name === 'distri')
      case 'local':
        return agents.filter((agent) => !agent.package_name)
      case 'plugin':
        return agents.filter((agent) => agent.package_name === packageFilter.name)
      default:
        return agents
    }
  }, [agents, packageFilter])

  const getAgentBadge = (agent: AgentDefinition) => {
    if (agent.package_name === 'distri') {
      return {
        label: 'Default',
        className: 'bg-blue-100 text-blue-800 dark:bg-blue-500/20 dark:text-blue-100 border-transparent'
      }
    }

    if (!agent.package_name) {
      return {
        label: 'Local',
        className: 'bg-amber-100 text-amber-900 dark:bg-amber-500/20 dark:text-amber-100 border-transparent'
      }
    }

    return {
      label: agent.package_name,
      className: 'bg-emerald-100 text-emerald-900 dark:bg-emerald-500/20 dark:text-emerald-100 border-transparent'
    }
  }

  const preferredAgent = useMemo(() => {
    if (!agents.length) {
      return undefined
    }
    return (
      agents.find((agent) => agent.id === 'scripter' || agent.name?.toLowerCase() === 'scripter') ||
      agents[0]
    )
  }, [agents])

  const handleHeroSend = (content: string | DistriPart[]) => {
    const text = typeof content === 'string'
      ? content
      : content.map(part => part.part_type === 'text' ? part.data : '').join('\n')

    if (!text.trim()) return
    if (preferredAgent && enableNavigation) {
      const params = new URLSearchParams()
      params.set('id', preferredAgent.id)
      params.set('prefill', text.trim())
      params.set('threadId', uuidv4())
      navigate(`/home/details?${params.toString()}`)
    }
    setHeroPrompt('')
    setShowCreator(false)
  }

  const handleChatWithAgent = (agent: AgentDefinition) => {
    if (onSelectAgent) {
      onSelectAgent(agent)
      return
    }
    if (!enableNavigation) {
      return
    }
    navigate(`/home/details?id=${encodeURIComponent(agent.id)}`)
  }

  const getAgentIcon = (agent: AgentDefinition) => {
    if (agent.agent_type === 'sequential_workflow_agent' ||
      agent.agent_type === 'dag_workflow_agent' ||
      agent.agent_type === 'custom_agent') {
      return <Workflow className="h-6 w-6 text-primary" />
    }
    return <Bot className="h-6 w-6 text-primary" />
  }

  const getActionButton = (agent: AgentDefinition) => {
    return (
      <Button
        size="sm"
        variant="ghost"
        onClick={() => handleChatWithAgent(agent)}
        className="h-8 w-8 p-0 rounded-lg hover:bg-muted flex-shrink-0"
        title="Open agent"
      >
        <ArrowRight className="h-4 w-4" />
      </Button>
    )
  }

  const shouldShowCreator = allowInlineCreator && showCreator

  return (
    <div className="flex h-full w-full overflow-hidden bg-background text-foreground">
      <div className="flex-1 overflow-auto">
        <div className="mx-auto flex w-full max-w-6xl flex-col gap-8 px-4 py-6 sm:px-6 lg:px-8 lg:py-10">
          {shouldShowCreator ? (
            <section className="rounded-2xl border border-border bg-card p-4 shadow-sm sm:p-6">
              <div className="mb-3 flex items-center justify-between">
                <div>
                  <p className="text-xs uppercase tracking-[0.35em] text-muted-foreground">New agent</p>
                  <h3 className="text-lg font-semibold">Describe the workflow you want</h3>
                </div>

              </div>
              <ChatInput
                value={heroPrompt}
                onChange={setHeroPrompt}
                onSend={handleHeroSend}
                placeholder="Describe the agent’s goals, inputs, and tools."
                variant="hero"
              />
            </section>
          ) : null}

          <section className="flex flex-col gap-6">
            <div className="flex flex-col gap-2">
              <div className="flex flex-wrap items-center justify-between gap-4">
                <div>
                  <p className="text-xs uppercase tracking-[0.35em]">Distri Agents</p>
                </div>
                <div className="flex flex-wrap gap-2">
                  <Button
                    size="sm"
                    className="gap-2"
                    onClick={() => {
                      if (allowInlineCreator) {
                        setShowCreator(true)
                      } else {
                        onRequestCreateAgent?.()
                      }
                    }}
                  >
                    Create new agent
                  </Button>
                </div>
              </div>
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  variant={packageFilter.kind === 'all' ? 'default' : 'outline'}
                  onClick={() => setPackageFilter({ kind: 'all' })}
                  className="text-xs"
                >
                  All
                </Button>
                {packageSummary.hasDefault && (
                  <Button
                    size="sm"
                    variant={packageFilter.kind === 'default' ? 'default' : 'outline'}
                    onClick={() => setPackageFilter({ kind: 'default' })}
                    className="text-xs"
                  >
                    Default
                  </Button>
                )}
                {packageSummary.hasLocal && (
                  <Button
                    size="sm"
                    variant={packageFilter.kind === 'local' ? 'default' : 'outline'}
                    onClick={() => setPackageFilter({ kind: 'local' })}
                    className="text-xs"
                  >
                    Local
                  </Button>
                )}
                {packageSummary.pluginNames.map((pkg) => (
                  <Button
                    key={pkg}
                    size="sm"
                    variant={packageFilter.kind === 'plugin' && packageFilter.name === pkg ? 'default' : 'outline'}
                    onClick={() => setPackageFilter({ kind: 'plugin', name: pkg })}
                    className="text-xs"
                  >
                    {pkg}
                  </Button>
                ))}
              </div>
            </div>

            {loading ? (
              <div className="flex justify-center py-12">
                <Skeleton className="h-40 w-full" />
              </div>
            ) : agents.length === 0 ? (
              <div className="text-center text-muted-foreground">No agents found. Create a new one to get started.</div>
            ) : filteredAgents.length === 0 ? (
              <div className="text-center text-muted-foreground">No agents found for the selected package.</div>
            ) : (
              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
                {filteredAgents.map((agent: AgentDefinition) => (
                  <Card
                    key={agent.id}
                    className="group border border-border/70 bg-card/95 text-foreground shadow-none transition hover:-translate-y-1 hover:border-primary/40 hover:shadow-lg"
                  >
                    <CardContent className="p-5">
                      <div className="flex flex-col gap-4">
                        <div className="flex items-center justify-between gap-3">
                          <div className="flex flex-1 items-center gap-3 min-w-0">
                            <div
                              className="flex h-12 w-12 cursor-pointer items-center justify-center rounded-2xl bg-primary/10 text-primary dark:bg-primary/20"
                              onClick={() => handleChatWithAgent(agent)}
                            >
                              <Avatar>
                                <AvatarImage src={agent.icon_url} />
                                <AvatarFallback className="bg-transparent text-primary">
                                  {getAgentIcon(agent)}
                                </AvatarFallback>
                              </Avatar>
                            </div>
                            <div className="min-w-0">
                              <div className="flex flex-wrap items-center gap-2">
                                <p
                                  className="cursor-pointer text-sm font-semibold text-foreground truncate hover:underline"
                                  title={agent.name}
                                  onClick={() => handleChatWithAgent(agent)}
                                >
                                  {agent.name}
                                </p>
                                {(() => {
                                  const badge = getAgentBadge(agent)
                                  return (
                                    <Badge variant="secondary" className={`text-xs ${badge.className}`}>
                                      {badge.label}
                                    </Badge>
                                  )
                                })()}
                              </div>
                              <p className="text-xs capitalize text-muted-foreground truncate" title={agent.agent_type?.replace(/_/g, ' ') || 'standard'}>
                                {agent.agent_type?.replace(/_/g, ' ') || 'standard'}
                              </p>
                            </div>
                          </div>
                          {getActionButton(agent)}
                        </div>

                        <p className="line-clamp-1 text-sm text-muted-foreground">{agent.description}</p>
                      </div>
                    </CardContent>
                  </Card>
                ))}
              </div>
            )}
          </section>
        </div>
      </div>
      {showDesignerChatProp && showDesignerChat && designerThreadIdRef.current ? (
        <DesignerSidebar
          threadId={designerThreadIdRef.current}
          onClose={() => setShowDesignerChat(false)}
        />
      ) : null}
    </div>
  )
}

interface DesignerSidebarProps {
  threadId: string
  onClose: () => void
}

const DesignerSidebar = ({ threadId, onClose }: DesignerSidebarProps) => {
  const { agent: designerAgent, loading } = useAgent({ agentIdOrDef: 'designer' })
  return (
    <aside className="w-80 border-l border-border bg-muted/10 p-2 dark:bg-muted/20">
      <div className="mb-2 flex items-center justify-between px-1">
        <p className="text-[11px] uppercase tracking-[0.35em] text-muted-foreground">Designer</p>
        <Button variant="ghost" size="sm" className="text-xs" onClick={onClose}>
          Hide
        </Button>
      </div>
      <div className="h-[calc(100%-1.75rem)] overflow-hidden">
        {designerAgent ? (
          <Chat agent={designerAgent} threadId={threadId} theme="auto" />
        ) : (
          <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
            {loading ? 'Loading designer…' : 'Designer unavailable'}
          </div>
        )}
      </div>
    </aside>
  )
}

export default AgentsHome
