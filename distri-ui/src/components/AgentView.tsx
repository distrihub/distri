import { useEffect, useMemo, useState, type CSSProperties, type ReactNode } from 'react'
import { useParams, useNavigate, useSearchParams } from 'react-router-dom'
import Editor from '@monaco-editor/react'
import { Chat, useAgent, useTheme } from '@distri/react'
import { FileText, Loader2, MessageCircle, X, Globe, Wrench, ChevronRight } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { BACKEND_URL } from '@/constants'
import { useInitialization } from '@/components/TokenProvider'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
} from '@/components/ui/sidebar'

const currentThreadId = (scope: string) => {
  if (typeof window === 'undefined') {
    return crypto.randomUUID()
  }
  const storageKey = `${scope}:threadId`
  const cached = window.localStorage.getItem(storageKey)
  if (cached) return cached
  const generated = crypto.randomUUID()
  window.localStorage.setItem(storageKey, generated)
  return generated
}

interface AgentDefinitionEnvelope {
  definition?: any
  markdown?: string
  [key: string]: any
}

interface AgentViewProps {
  defaultChatOpen?: boolean
}

export default function AgentView({ defaultChatOpen = false }: AgentViewProps) {
  const { agentId: encodedAgentId } = useParams<{ agentId: string }>()
  const [searchParams] = useSearchParams()
  const queryAgentId = searchParams.get('id')
  const agentId = encodedAgentId ? decodeURIComponent(encodedAgentId) : (queryAgentId || undefined)
  const queryThreadId = searchParams.get('threadId')

  const { agent, loading: agentLoading } = useAgent({ agentIdOrDef: agentId || '' })
  const { token } = useInitialization()
  const { setTheme } = useTheme()
  const navigate = useNavigate()

  const [definition, setDefinition] = useState<AgentDefinitionEnvelope | null>(null)
  const [sourceLoading, setSourceLoading] = useState(false)
  const [definitionOpen, setDefinitionOpen] = useState(false)
  const [chatOpen, setChatOpen] = useState(defaultChatOpen)
  const [toolsOpen, setToolsOpen] = useState(false)
  const [activeSample, setActiveSample] = useState<'curl' | 'node' | 'python' | 'react'>('curl')
  const [initialMessages, setInitialMessages] = useState<any[] | undefined>(undefined)

  const threadId = useMemo(
    () => {
      if (queryThreadId) return queryThreadId;
      return currentThreadId(agentId ? `agent:${agentId}` : 'agent')
    },
    [agentId, queryThreadId],
  )

  const agentType = agent?.getDefinition?.().agent_type ?? agent?.agentType

  const agentDefinition = useMemo(() => {
    if (definition?.agent) return definition.agent
    if (definition?.definition) return definition.definition
    if (definition && (definition as any).name) return definition
    return agent?.getDefinition?.()
  }, [agent, definition])

  const toolDefinitions = useMemo(() => {
    const tools = definition?.tools ?? agentDefinition?.tools
    return Array.isArray(tools) ? tools : []
  }, [definition, agentDefinition])

  const toolRows = useMemo(() => {
    return toolDefinitions.map((tool: any) => {
      const name = tool?.name ?? tool?.function?.name ?? tool?.id ?? 'unknown_tool'
      const description =
        tool?.description ??
        tool?.function?.description ??
        tool?.metadata?.description ??
        ''
      return { name, description }
    })
  }, [toolDefinitions])

  const externalToolValidation = useMemo(() => {
    if (!agent) {
      return {
        isValid: true,
        requiredTools: [],
        providedTools: [],
        missingTools: [],
        message: undefined,
      }
    }
    return agent.validateExternalTools()
  }, [agent])

  const hasExternalTools = externalToolValidation.requiredTools.length > 0
  const chatDisabled = hasExternalTools
  const embeddedAgentMessage =
    'Agent has external tools. This is an embedded Agent that can run within the parent application. Register DistriWidget for embedding the parent component.'

  useEffect(() => {
    setTheme?.('dark')
  }, [setTheme])

  useEffect(() => {
    if (!agentId) {
      return
    }
    const load = async () => {
      setSourceLoading(true)
      try {
        const headers: Record<string, string> = {}
        if (token) {
          headers['Authorization'] = `Bearer ${token}`
        }
        const resp = await fetch(`${BACKEND_URL}/v1/agents/${encodeURIComponent(agentId)}`, {
          headers,
        })
        if (!resp.ok) {
          const message = await resp.text()
          throw new Error(message || `Failed to load agent ${agentId}`)
        }
        const data: AgentDefinitionEnvelope = await resp.json()
        setDefinition(data)
      } catch (err) {
        console.error(err)
      } finally {
        setSourceLoading(false)
      }
    }
    void load()
  }, [agentId, token])

  useEffect(() => {
    if (!threadId) {
      setInitialMessages(undefined)
      return
    }
    const loadMessages = async () => {
      try {
        const headers: Record<string, string> = {}
        if (token) {
          headers['Authorization'] = `Bearer ${token}`
        }
        const resp = await fetch(`${BACKEND_URL}/v1/threads/${encodeURIComponent(threadId)}/messages`, {
          headers,
        })
        if (resp.ok) {
          const data = await resp.json()
          const converted = Array.isArray(data)
            ? data.map((msg: any) => ({
              ...msg,
              id: msg.id || crypto.randomUUID(),
              role: msg.role === 'agent' ? 'assistant' : msg.role,
              content:
                typeof msg.content === 'string'
                  ? msg.content
                  : JSON.stringify(msg.content),
            }))
            : []
          setInitialMessages(converted)
        } else {
          setInitialMessages([])
        }
      } catch (err) {
        console.error('Failed to load thread messages', err)
        setInitialMessages([])
      }
    }
    void loadMessages()
  }, [threadId, token])

  if (agentLoading || sourceLoading) {
    return (
      <div className="flex h-full items-center justify-center bg-slate-950">
        <div className="flex items-center gap-3 text-slate-200">
          <Loader2 className="h-5 w-5 animate-spin text-slate-400" />
          Loading agent…
        </div>
      </div>
    )
  }

  if (!agent) {
    return (
      <div className="flex h-full items-center justify-center bg-slate-950 text-slate-300 px-4">
        <div className="flex max-w-md flex-col items-center text-center gap-2">
          <p className="text-lg font-semibold">Agent not found</p>
          <p className="text-sm text-slate-500">Check the URL or create a new agent.</p>
        </div>
      </div>
    )
  }

  const displayName = agentDefinition?.name ?? agent?.name ?? agentId ?? 'Agent'
  const description = agentDefinition?.description ?? agent?.description ?? ''
  const packageName = agentDefinition?.package_name
  const version = agentDefinition?.version
  const modelName = agentDefinition?.model_settings?.model
  const analysisModelName = agentDefinition?.analysis_model_settings?.model
  const maxIterations = agentDefinition?.max_iterations
  const historySize = agentDefinition?.history_size
  const contextSize =
    agentDefinition?.context_size ?? agentDefinition?.model_settings?.context_size
  const browserEnabled = Boolean(agentDefinition?.browser_config?.enabled)
  const subAgents = Array.isArray(agentDefinition?.sub_agents)
    ? agentDefinition?.sub_agents
    : []
  const skillCount = agentDefinition?.skills?.length ?? 0
  const definitionJson = JSON.stringify(agentDefinition ?? {}, null, 2)
  const definitionMarkdown = definition?.markdown ?? ''
  const agentFilterId = agentDefinition?.id ?? agentId ?? displayName
  const sampleAgentRef = agentDefinition?.id ?? agentId ?? 'agent_id'
  const sampleBaseUrl = `${BACKEND_URL}/v1`
  const sampleSnippets = {
    curl: [
      `curl -X POST "${sampleBaseUrl}/agents/${sampleAgentRef}/invoke" \\`,
      `  -H "Content-Type: application/json" \\`,
      `  -d '{ "input": "Hello, agent!" }'`,
    ].join('\\n'),
    node: [
      `import fetch from 'node-fetch'`,
      ``,
      `const res = await fetch("${sampleBaseUrl}/agents/${sampleAgentRef}/invoke", {`,
      `  method: "POST",`,
      `  headers: { "Content-Type": "application/json" },`,
      `  body: JSON.stringify({ input: "Hello, agent!" })`,
      `})`,
      `const data = await res.json()`,
      `console.log(data)`,
    ].join('\\n'),
    python: [
      `import requests`,
      ``,
      `resp = requests.post("${sampleBaseUrl}/agents/${sampleAgentRef}/invoke",`,
      `  json={"input": "Hello, agent!"})`,
      `print(resp.json())`,
    ].join('\\n'),
    react: [
      `import { useAgent, Chat } from "@distri/react"`,
      ``,
      `const MyAgentChat = () => {`,
      `  const { agent } = useAgent({ agentIdOrDef: "${sampleAgentRef}" })`,
      `  return agent ? <Chat agent={agent} threadId="my-thread" theme="dark" /> : null`,
      `}`,
    ].join('\\n'),
  }
  const actionBarStyles: CSSProperties = {
    '--sidebar-width': '3.5rem',
    '--sidebar-width-mobile': '3.5rem',
  } as CSSProperties
  const sidePanelWidth = 'min(40vw, 720px)'

  return (
    <TooltipProvider>
      <SidebarProvider style={actionBarStyles}>
        <div className="relative flex h-full w-full flex-col bg-slate-950 text-slate-50 overflow-hidden lg:flex-row">
          <div className="flex-1 min-w-0 p-3 sm:p-4">
            <div className="h-full overflow-y-auto pr-1">
              <div className="flex flex-col gap-4 max-w-5xl mx-auto">
                <div className="flex flex-col gap-4">
                  <div className="rounded-lg border border-slate-900/60 bg-slate-950/70 p-5">
                    <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
                      <div className="space-y-2">
                        <p className="text-[11px] uppercase tracking-[0.2em] text-slate-500">Agent</p>
                        <div className="flex items-center gap-2">
                          <Button
                            variant="ghost"
                            className="h-auto p-0 text-2xl font-semibold text-slate-50 hover:text-slate-200 hover:bg-transparent"
                            onClick={() => {
                              setChatOpen(false)
                              setToolsOpen(false)
                              setDefinitionOpen((open) => !open)
                            }}
                          >
                            {displayName}
                          </Button>
                          <div className="flex items-center">
                            <Button
                              variant="ghost"
                              size="icon"
                              className="h-8 w-8 text-slate-400 hover:text-slate-100"
                              onClick={() => {
                                setChatOpen(false)
                                setToolsOpen(false)
                                setDefinitionOpen((open) => !open)
                              }}
                              title="Toggle Definition"
                            >
                              <FileText className="h-4 w-4" />
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon"
                              className="h-8 w-8 text-slate-400 hover:text-slate-100"
                              onClick={() => {
                                setDefinitionOpen(false)
                                setToolsOpen(false)
                                setChatOpen((open) => !open)
                              }}
                              title="Toggle Chat"
                            >
                              <MessageCircle className="h-4 w-4" />
                            </Button>
                          </div>
                        </div>
                        <p className="text-sm text-slate-400">
                          {description || 'No description provided.'}
                        </p>
                        <div className="flex flex-wrap items-center gap-3 text-xs">
                          <Button
                            variant="link"
                            size="sm"
                            className="h-7 px-0"
                            onClick={() =>
                              navigate(
                                `/home/threads?agent=${encodeURIComponent(agentFilterId)}`,
                              )
                            }
                          >
                            Recent Threads
                          </Button>
                        </div>
                      </div>
                      <div className="flex flex-wrap gap-2 text-xs text-slate-200">
                        <DetailBadge label="Type" value={formatAgentType(agentType)} />
                        {packageName ? <DetailBadge label="Package" value={packageName} /> : null}
                        {version ? <DetailBadge label="Version" value={version} /> : null}
                      </div>
                    </div>

                    <div className="mt-4 grid gap-3">
                      <DetailRow label="Agent ID" value={agentId || agentDefinition?.id || agent?.id} />
                      <DetailRow
                        label="Tools"
                        value={
                          toolRows.length ? (
                            <Button
                              variant="ghost"
                              size="sm"
                              className="h-auto p-0 text-slate-100 hover:text-white flex items-center gap-1.5"
                              onClick={() => {
                                setChatOpen(false)
                                setDefinitionOpen(false)
                                setToolsOpen(true)
                              }}
                            >
                              <span>{toolRows.length}</span>
                              {externalToolValidation.requiredTools.length > 0 && (
                                <div className="flex items-center gap-1 text-amber-400">
                                  <span>(</span>
                                  <Tooltip>
                                    <TooltipTrigger asChild>
                                      <Globe className="h-3 w-3" />
                                    </TooltipTrigger>
                                    <TooltipContent>Externally Configured Tool</TooltipContent>
                                  </Tooltip>
                                  <span>{externalToolValidation.requiredTools.length}</span>
                                  <span>)</span>
                                </div>
                              )}
                              <ChevronRight className="h-3 w-3 text-slate-500" />
                            </Button>
                          ) : (
                            'None'
                          )
                        }
                      />

                      <DetailRow
                        label="Sub-agents"
                        value={
                          subAgents.length ? (
                            <div className="flex flex-wrap justify-end gap-2">
                              {subAgents.map((subAgent: any) => (
                                <Button
                                  key={subAgent}
                                  variant="outline"
                                  size="sm"
                                  className="h-7 px-2 text-xs"
                                  onClick={() =>
                                    navigate(
                                      `/home/chat?id=${encodeURIComponent(subAgent)}`,
                                    )
                                  }
                                >
                                  {subAgent}
                                </Button>
                              ))}
                            </div>
                          ) : (
                            'None'
                          )
                        }
                      />
                      <DetailRow label="Skills" value={skillCount ? skillCount : 'None'} />
                    </div>
                  </div>

                  <DetailCard title="Runtime">
                    <DetailRow label="Model" value={modelName || 'Default'} />
                    <DetailRow label="Analysis model" value={analysisModelName || 'Default'} />
                    <DetailRow label="Max iterations" value={maxIterations ?? 'Default'} />
                    <DetailRow label="History size" value={historySize ?? 'Default'} />
                    <DetailRow label="Context size" value={contextSize ?? 'Default'} />
                    <DetailRow label="Browser" value={browserEnabled ? 'Enabled' : 'Disabled'} />
                  </DetailCard>

                  <DetailCard title="Code Samples">
                    <p className="text-sm text-slate-400">Call this agent from your stack.</p>
                    <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-slate-400">
                      <span>Base URL: {sampleBaseUrl}</span>
                      <span className="rounded-md border border-slate-800/70 bg-slate-900/60 px-2 py-1">
                        Agent: {sampleAgentRef}
                      </span>
                    </div>
                    <Tabs
                      value={activeSample}
                      onValueChange={(value) =>
                        setActiveSample(value as typeof activeSample)
                      }
                    >
                      <TabsList className="bg-slate-900">
                        <TabsTrigger value="curl">cURL</TabsTrigger>
                        <TabsTrigger value="node">Node</TabsTrigger>
                        <TabsTrigger value="python">Python</TabsTrigger>
                        <TabsTrigger value="react">React</TabsTrigger>
                      </TabsList>

                      <TabsContent value="curl" className="pt-3">
                        <SampleEditor value={sampleSnippets.curl} language="shell" />
                      </TabsContent>
                      <TabsContent value="node" className="pt-3">
                        <SampleEditor value={sampleSnippets.node} language="typescript" />
                      </TabsContent>
                      <TabsContent value="python" className="pt-3">
                        <SampleEditor value={sampleSnippets.python} language="python" />
                      </TabsContent>
                      <TabsContent value="react" className="pt-3">
                        <SampleEditor value={sampleSnippets.react} language="typescript" />
                      </TabsContent>
                    </Tabs>
                  </DetailCard>
                </div>
              </div>
            </div>
          </div>

          <Sidebar
            side="right"
            variant="floating"
            collapsible="none"
            className="w-[--sidebar-width] border-l border-slate-800/70 bg-slate-900/80 text-slate-50 shadow-xl"
          >
            <SidebarContent className="h-full p-2 pt-3">
              <SidebarGroup>
                <SidebarGroupContent>
                  <SidebarMenu>
                    <SidebarMenuItem>
                      <SidebarMenuButton
                        isActive={chatOpen}
                        className="justify-start gap-3"
                        onClick={() => {
                          setDefinitionOpen(false)
                          setToolsOpen(false)
                          setChatOpen((open) => !open)
                        }}
                        title="Toggle chat"
                      >
                        <MessageCircle className="h-4 w-4" />
                        <span className="text-sm">Chat</span>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                    <SidebarMenuItem>
                      <SidebarMenuButton
                        isActive={definitionOpen}
                        className="justify-start gap-3"
                        onClick={() => {
                          setChatOpen(false)
                          setToolsOpen(false)
                          setDefinitionOpen((open) => !open)
                        }}
                        title="Toggle definition"
                      >
                        <FileText className="h-4 w-4" />
                        <span className="text-sm">Definition</span>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                    <SidebarMenuItem>
                      <SidebarMenuButton
                        isActive={toolsOpen}
                        className="justify-start gap-3"
                        onClick={() => {
                          setChatOpen(false)
                          setDefinitionOpen(false)
                          setToolsOpen((open) => !open)
                        }}
                        title="Toggle tools"
                      >
                        <Wrench className="h-4 w-4" />
                        <span className="text-sm">Tools</span>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  </SidebarMenu>
                </SidebarGroupContent>
              </SidebarGroup>
            </SidebarContent>
          </Sidebar>

          {definitionOpen ? (
            <div
              className="z-20 flex h-full w-[min(40vw,720px)] flex-col border-l border-slate-800 bg-slate-950 text-slate-50 shadow-2xl lg:absolute lg:right-[3.5rem] lg:top-0"
              style={{ width: sidePanelWidth }}
            >
              <div className="flex items-center justify-between border-b border-slate-800 px-4 py-3">
                <div>
                  <p className="text-[11px] uppercase tracking-[0.2em] text-slate-500">Definition</p>
                  <p className="text-[11px] text-slate-500">
                    {definitionMarkdown.trim()
                      ? 'Agent markdown source.'
                      : 'Agent JSON configuration.'}
                  </p>
                </div>
                <Button
                  size="icon"
                  variant="ghost"
                  onClick={() => setDefinitionOpen(false)}
                  className="text-slate-300 hover:text-white"
                >
                  <X className="h-4 w-4" />
                </Button>
              </div>
              <div className="flex-1 min-h-0 overflow-hidden p-3">
                <div className="h-full w-full overflow-hidden rounded-lg border border-slate-900/60 bg-slate-950/70">
                  <Editor
                    height="100%"
                    value={definitionMarkdown.trim() ? definitionMarkdown : definitionJson}
                    defaultLanguage={definitionMarkdown.trim() ? 'markdown' : 'json'}
                    theme="vs-dark"
                    options={{
                      fontSize: 13,
                      minimap: { enabled: false },
                      scrollBeyondLastLine: false,
                      readOnly: true,
                      wordWrap: 'on',
                      padding: { top: 12, bottom: 12 },
                    }}
                  />
                </div>
              </div>
            </div>
          ) : null}

          {toolsOpen ? (
            <div
              className="z-20 flex h-full w-[min(40vw,720px)] flex-col border-l border-slate-800 bg-slate-950 text-slate-50 shadow-2xl lg:absolute lg:right-[3.5rem] lg:top-0"
              style={{ width: sidePanelWidth }}
            >
              <div className="flex items-center justify-between border-b border-slate-800 px-4 py-3">
                <div>
                  <p className="text-[11px] uppercase tracking-[0.2em] text-slate-500">Tools</p>
                  <p className="text-[11px] text-slate-500">Registered tools.</p>
                </div>
                <Button
                  size="icon"
                  variant="ghost"
                  onClick={() => setToolsOpen(false)}
                  className="text-slate-300 hover:text-white"
                >
                  <X className="h-4 w-4" />
                </Button>
              </div>
              <div className="flex-1 min-h-0 overflow-y-auto p-3">
                {hasExternalTools ? (
                  <div className="mt-2 space-y-2 mb-4">
                    <p className="text-[11px] uppercase tracking-[0.2em] text-slate-500">
                      External tools required
                    </p>
                    <div className="flex flex-wrap gap-2">
                      {externalToolValidation.requiredTools.map((tool) => (
                        <span
                          key={tool}
                          className="rounded-md border border-amber-500/40 bg-amber-500/10 px-2 py-1 text-xs text-amber-100"
                        >
                          {tool}
                        </span>
                      ))}
                    </div>
                  </div>
                ) : null}

                {toolRows.length ? (
                  <div className="overflow-hidden rounded-md border border-slate-800/70">
                    <table className="w-full table-fixed text-left text-sm">
                      <colgroup>
                        <col className="w-[35%]" />
                        <col className="w-[65%]" />
                      </colgroup>
                      <thead className="bg-slate-900/60 text-slate-400">
                        <tr>
                          <th className="px-3 py-2 text-[11px] font-medium uppercase tracking-[0.2em]">
                            Name
                          </th>
                          <th className="px-3 py-2 text-[11px] font-medium uppercase tracking-[0.2em]">
                            Description
                          </th>
                        </tr>
                      </thead>
                      <tbody>
                        {toolRows.map((tool, index) => {
                          const isExternal = externalToolValidation.requiredTools.includes(
                            tool.name,
                          )
                          return (
                            <tr
                              key={`${tool.name}-${index}`}
                              className="border-t border-slate-800/70"
                            >
                              <td className="px-3 py-2 text-slate-100 truncate flex items-center gap-2">
                                <span title={tool.name} className="truncate">
                                  {tool.name}
                                </span>
                                {isExternal && (
                                  <Tooltip>
                                    <TooltipTrigger asChild>
                                      <Globe className="h-3 w-3 text-amber-400 shrink-0" />
                                    </TooltipTrigger>
                                    <TooltipContent>External Tool</TooltipContent>
                                  </Tooltip>
                                )}
                              </td>
                              <td
                                className="px-3 py-2 text-slate-400 truncate"
                                title={tool.description || 'No description'}
                              >
                                {tool.description || 'No description'}
                              </td>
                            </tr>
                          )
                        })}
                      </tbody>
                    </table>
                  </div>
                ) : (
                  <span className="text-sm text-slate-500">No tools registered.</span>
                )}
              </div>
            </div>
          ) : null}

          {chatOpen ? (
            <div
              className="z-20 flex h-full w-[min(40vw,720px)] flex-col border-l border-slate-800 bg-slate-950 text-slate-50 shadow-2xl lg:absolute lg:right-[3.5rem] lg:top-0"
              style={{ width: sidePanelWidth }}
            >
              <div className="flex items-center justify-between border-b border-slate-800 px-4 py-3">
                <div>
                  <p className="text-[11px] uppercase tracking-[0.2em] text-slate-500">Chat</p>
                  <p className="text-[11px] text-slate-500">Talk to this agent.</p>
                </div>
                <Button
                  size="icon"
                  variant="ghost"
                  onClick={() => setChatOpen(false)}
                  className="text-slate-300 hover:text-white"
                >
                  <X className="h-4 w-4" />
                </Button>
              </div>
              <div className="flex-1 min-h-0 overflow-hidden p-3">
                <div className="h-full w-full overflow-hidden rounded-lg border border-slate-900/60 bg-slate-950/70">
                  {chatDisabled ? (
                    <div className="flex h-full w-full items-center justify-center p-6">
                      <div className="max-w-md rounded-lg border border-amber-500/40 bg-amber-500/10 p-4 text-sm text-amber-100">
                        <p className="text-sm font-semibold text-amber-200">Chat disabled</p>
                        <p className="mt-2 text-sm text-amber-100/90">
                          {embeddedAgentMessage}
                        </p>
                        {externalToolValidation.requiredTools.length ? (
                          <div className="mt-3 flex flex-wrap gap-2 text-xs text-amber-100/90">
                            {externalToolValidation.requiredTools.map((tool) => (
                              <span
                                key={tool}
                                className="rounded-md border border-amber-500/40 bg-amber-500/10 px-2 py-1"
                              >
                                {tool}
                              </span>
                            ))}
                          </div>
                        ) : null}
                      </div>
                    </div>
                  ) : (
                    initialMessages === undefined ? (
                      <div className="flex h-full items-center justify-center p-6 text-slate-400 gap-2">
                        <Loader2 className="animate-spin h-5 w-5" />
                        Loading chat...
                      </div>
                    ) : (
                      <Chat key={threadId} agent={agent} threadId={threadId} initialMessages={initialMessages} theme="dark" />
                    )
                  )}
                </div>
              </div>
            </div>
          ) : null}
        </div>
      </SidebarProvider>
    </TooltipProvider>
  )
}

type DetailCardProps = {
  title: string
  children: ReactNode
  className?: string
}

const DetailCard = ({ title, children, className }: DetailCardProps) => {
  return (
    <div className={`rounded-lg border border-slate-900/60 bg-slate-950/70 p-4 ${className ?? ''}`}>
      <p className="text-[11px] uppercase tracking-[0.2em] text-slate-500">{title}</p>
      <div className="mt-3 flex flex-col gap-2">{children}</div>
    </div>
  )
}

type DetailRowProps = {
  label: string
  value?: ReactNode
}

const DetailRow = ({ label, value }: DetailRowProps) => {
  return (
    <div className="flex items-start justify-between gap-3 text-sm">
      <span className="text-slate-500">{label}</span>
      <div className="text-right text-slate-100 break-all">{value ?? '--'}</div>
    </div>
  )
}

type DetailBadgeProps = {
  label: string
  value: ReactNode
}

const DetailBadge = ({ label, value }: DetailBadgeProps) => {
  return (
    <span className="rounded-md border border-slate-800/80 bg-slate-900/60 px-2 py-1 text-[11px] text-slate-200">
      {label}: <span className="text-slate-100">{value}</span>
    </span>
  )
}

const formatAgentType = (value?: string) => {
  if (!value) return 'Standard Agent'
  return value
    .replace(/_/g, ' ')
    .replace(/\b\w/g, (match) => match.toUpperCase())
}

const SampleEditor = ({ value, language }: { value: string; language?: string }) => {
  return (
    <div className="h-[300px] min-h-[220px] overflow-hidden rounded-md border border-slate-800/70 bg-slate-950/80">
      <Editor
        height="100%"
        value={value}
        defaultLanguage={language || 'plaintext'}
        language={language || 'plaintext'}
        theme="vs-dark"
        options={{
          fontSize: 14,
          minimap: { enabled: false },
          scrollBeyondLastLine: false,
          readOnly: true,
          wordWrap: 'on',
          padding: { top: 12, bottom: 12 },
        }}
      />
    </div>
  )
}
