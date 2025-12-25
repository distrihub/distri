import { useEffect, useMemo, useState, type CSSProperties } from 'react'
import { useParams, useNavigate } from 'react-router-dom'
import Editor from '@monaco-editor/react'
import { Chat, useAgent, useTheme } from '@distri/react'
import { uuidv4 } from '@distri/core'
import { Loader2, FileText, X, CopyPlus, Code2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { WorkflowDetailsView } from '@/components/WorkflowDetailsView'
import { SkeletonCard } from '@/components/ReplayChat'
import { BACKEND_URL } from '@/constants'
import { toast } from 'sonner'
import { useInitialization } from '@/components/TokenProvider'
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
} from '@/components/ui/sidebar'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'

const currentThreadId = (scope: string) => {
  if (typeof window === 'undefined') {
    return uuidv4()
  }
  const storageKey = `${scope}:threadId`
  const cached = window.localStorage.getItem(storageKey)
  if (cached) return cached
  const generated = uuidv4()
  window.localStorage.setItem(storageKey, generated)
  return generated
}

interface AgentDefinitionEnvelope {
  definition?: any
  markdown?: string
  [key: string]: any
}

export default function AgentDetailsPage() {
  const { agentId: encodedAgentId } = useParams<{ agentId: string }>()
  const agentId = encodedAgentId ? decodeURIComponent(encodedAgentId) : undefined
  const { agent, loading: agentLoading } = useAgent({ agentIdOrDef: agentId || '' })
  const { token } = useInitialization()
  const { setTheme } = useTheme()
  const navigate = useNavigate()

  const [markdown, setMarkdown] = useState<string>('')
  const [initialMarkdown, setInitialMarkdown] = useState<string>('')
  const [definition, setDefinition] = useState<AgentDefinitionEnvelope | null>(null)
  const [sourceLoading, setSourceLoading] = useState(false)
  const [sourceError, setSourceError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [codeOpen, setCodeOpen] = useState(false)
  const [samplesOpen, setSamplesOpen] = useState(false)

  const threadId = useMemo(
    () => currentThreadId(agentId ? `agent:${agentId}` : 'agent'),
    [agentId],
  )

  const agentType = agent?.getDefinition?.().agent_type ?? agent?.agentType
  const isWorkflowAgent =
    agentType === 'sequential_workflow_agent' ||
    agentType === 'dag_workflow_agent' ||
    agentType === 'custom_agent'

  useEffect(() => {
    setTheme?.('dark')
  }, [setTheme])

  useEffect(() => {
    if (!agentId) {
      return
    }
    const load = async () => {
      setSourceLoading(true)
      setSourceError(null)
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
        const definitionPayload = data.definition ?? data
        const rawMarkdown = typeof data.markdown === 'string' ? data.markdown : ''

        if (!rawMarkdown) {
          throw new Error('Backend did not return markdown for this agent')
        }

        setDefinition(definitionPayload)
        setMarkdown(rawMarkdown)
        setInitialMarkdown(rawMarkdown)
      } catch (err) {
        console.error(err)
        setSourceError(err instanceof Error ? err.message : 'Failed to load agent')
      } finally {
        setSourceLoading(false)
      }
    }
    void load()
  }, [agentId, token])

  const handleSave = async () => {
    if (!agentId) return
    setSaving(true)
    try {
      const headers: Record<string, string> = { 'Content-Type': 'text/plain' }
      if (token) {
        headers['Authorization'] = `Bearer ${token}`
      }
      const resp = await fetch(`${BACKEND_URL}/v1/agents/${encodeURIComponent(agentId)}`, {
        method: 'PUT',
        headers,
        body: markdown,
      })
      if (!resp.ok) {
        const message = await resp.text()
        throw new Error(message || 'Failed to save agent')
      }
      const data: AgentDefinitionEnvelope = await resp.json()
      const savedMarkdown = data.markdown ?? markdown
      setInitialMarkdown(savedMarkdown)
      setMarkdown(savedMarkdown)
      toast.success('Agent markdown saved')
    } catch (err) {
      console.error(err)
      toast.error(err instanceof Error ? err.message : 'Failed to save agent')
    } finally {
      setSaving(false)
    }
  }

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

  if (isWorkflowAgent) {
    return <WorkflowDetailsView agent={agent} />
  }

  const isDirty = markdown !== initialMarkdown
  const actionBarStyles: CSSProperties = {
    '--sidebar-width': '3.5rem',
    '--sidebar-width-mobile': '3.5rem',
  }
  const codePanelWidth = 'min(40vw, 720px)'

  return (
    <SidebarProvider style={actionBarStyles}>
      <div className="relative flex h-full w-full bg-slate-950 text-slate-50 overflow-hidden">
        <div className="flex-1 min-w-0 p-3 sm:p-4">
          <div className="h-full w-full overflow-hidden rounded-lg border border-slate-900/60 bg-slate-950/70">
            {agent ? (
              <Chat agent={agent} threadId={threadId} theme="dark" />
            ) : (
              <div className="flex h-full w-full items-center justify-center text-slate-400">
                <SkeletonCard />
              </div>
            )}
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
                      isActive={codeOpen}
                      className="justify-start gap-3"
                      onClick={() => {
                        setSamplesOpen(false)
                        setCodeOpen((open) => !open)
                      }}
                      title="Toggle definition"
                    >
                      <FileText className="h-4 w-4" />
                      <span className="text-sm">Definition</span>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                  <SidebarMenuItem>
                    <SidebarMenuButton
                      isActive={samplesOpen}
                      className="justify-start gap-3"
                      onClick={() => {
                        setCodeOpen(false)
                        setSamplesOpen((open) => !open)
                      }}
                      title="Toggle code samples"
                    >
                      <Code2 className="h-4 w-4" />
                      <span className="text-sm">Samples</span>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                </SidebarMenu>
              </SidebarGroupContent>
            </SidebarGroup>
          </SidebarContent>
        </Sidebar>

        {codeOpen ? (
          <div
            className="z-20 flex h-full w-[min(40vw,720px)] flex-col border-l border-slate-800 bg-slate-950 text-slate-50 shadow-2xl"
            style={{ width: codePanelWidth }}
          >
            <AgentSourceSidebar
              isDirty={isDirty}
              markdown={markdown}
              onChange={(value) => setMarkdown(value)}
              onClose={() => setCodeOpen(false)}
              onSave={() => void handleSave()}
              saving={saving}
              sourceError={sourceError}
              onClone={() =>
                handleClone({ agent, agentId, definition, navigate, markdown })
              }
            />
          </div>
        ) : null}

        {samplesOpen ? (
          <div
            className="z-20 flex h-full w-[min(40vw,720px)] flex-col border-l border-slate-800 bg-slate-950 text-slate-50 shadow-2xl"
            style={{ width: codePanelWidth }}
          >
            <AgentSamplesSidebar agentId={agentId} onClose={() => setSamplesOpen(false)} />
          </div>
        ) : null}
      </div>
    </SidebarProvider>
  )
}

type AgentSourceSidebarProps = {
  isDirty: boolean
  markdown: string
  onChange: (value: string) => void
  onClose: () => void
  onSave: () => void
  saving: boolean
  sourceError: string | null
  onClone: () => void
}

function AgentSourceSidebar({
  isDirty,
  markdown,
  onChange,
  onClose,
  onSave,
  saving,
  sourceError,
  onClone,
}: AgentSourceSidebarProps) {
  return (
    <div className="flex h-full flex-col bg-slate-950">
      <div className="flex items-center justify-between border-b border-slate-800 px-4 py-3">
        <div>
          <p className="text-[11px] uppercase tracking-[0.2em] text-slate-500">Agent Markdown</p>
          <p className="text-[11px] text-slate-500">Edit and save the source.</p>
        </div>
        <div className="flex items-center gap-2">
          <Button size="sm" variant="outline" onClick={onClone} className="gap-1">
            <CopyPlus className="h-4 w-4" />
            Clone
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={onSave}
            disabled={!isDirty || saving}
            className="gap-2"
          >
            {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
            Save
          </Button>
          <Button
            size="icon"
            variant="ghost"
            onClick={onClose}
            className="text-slate-300 hover:text-white"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      </div>

      {sourceError ? (
        <div className="mx-4 mt-2 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-200">
          {sourceError}
        </div>
      ) : null}

      <div className="flex-1 min-h-0 overflow-hidden p-3">
        <div className="flex-1 min-h-0 overflow-hidden rounded-md border border-slate-800/70 bg-slate-950/80">
          <Editor
            height="100%"
            value={markdown}
            defaultLanguage="markdown"
            theme="vs-dark"
            onChange={(value) => onChange(value ?? '')}
            options={{
              fontSize: 14,
              minimap: { enabled: false },
              scrollBeyondLastLine: false,
              wordWrap: 'on',
              padding: { top: 12, bottom: 12 },
            }}
          />
        </div>
      </div>
    </div>
  )
}

type AgentSamplesSidebarProps = {
  agentId?: string
  onClose: () => void
}

function AgentSamplesSidebar({ agentId, onClose }: AgentSamplesSidebarProps) {
  const [activeSample, setActiveSample] = useState<'curl' | 'node' | 'python' | 'react'>('curl')
  const baseUrl = `${BACKEND_URL}/api/v1`
  const agentRef = agentId || 'agent_id'
  const samples: Record<typeof activeSample, string> = {
    curl: [
      `curl -X POST "${baseUrl}/agents/${agentRef}/invoke" \\`,
      `  -H "Content-Type: application/json" \\`,
      `  -d '{ "input": "Hello, agent!" }'`,
    ].join('\n'),
    node: [
      `import fetch from 'node-fetch'`,
      ``,
      `const res = await fetch("${baseUrl}/agents/${agentRef}/invoke", {`,
      `  method: "POST",`,
      `  headers: { "Content-Type": "application/json" },`,
      `  body: JSON.stringify({ input: "Hello, agent!" })`,
      `})`,
      `const data = await res.json()`,
      `console.log(data)`,
    ].join('\n'),
    python: [
      `import requests`,
      ``,
      `resp = requests.post("${baseUrl}/agents/${agentRef}/invoke",`,
      `  json={"input": "Hello, agent!"})`,
      `print(resp.json())`,
    ].join('\n'),
    react: [
      `import { useAgent, Chat } from "@distri/react"`,
      ``,
      `const MyAgentChat = () => {`,
      `  const { agent } = useAgent({ agentIdOrDef: "${agentRef}" })`,
      `  return agent ? <Chat agent={agent} threadId="my-thread" theme="dark" /> : null`,
      `}`,
    ].join('\n'),
  }

  return (
    <div className="flex h-full flex-col bg-slate-950">
      <div className="flex items-center justify-between border-b border-slate-800 px-4 py-3">
        <div>
          <p className="text-[11px] uppercase tracking-[0.2em] text-slate-500">Code Samples</p>
          <p className="text-[11px] text-slate-500">Call this agent from your stack.</p>
        </div>
        <Button
          size="icon"
          variant="ghost"
          onClick={onClose}
          className="text-slate-300 hover:text-white"
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      <div className="flex items-center justify-between px-4 py-3">
        <div className="text-sm text-slate-300">Base URL: {baseUrl}</div>
        <div className="rounded-md border border-slate-800 bg-slate-900/70 px-2 py-1 text-[11px] text-slate-400">
          Agent: {agentRef}
        </div>
      </div>

      <div className="flex-1 min-h-0 px-3 pb-3">
        <Tabs value={activeSample} onValueChange={(val) => setActiveSample(val as typeof activeSample)}>
          <TabsList className="bg-slate-900">
            <TabsTrigger value="curl">cURL</TabsTrigger>
            <TabsTrigger value="node">Node</TabsTrigger>
            <TabsTrigger value="python">Python</TabsTrigger>
            <TabsTrigger value="react">React</TabsTrigger>
          </TabsList>

          <TabsContent value="curl" className="flex-1 min-h-0 pt-3">
            <SampleEditor value={samples.curl} language="shell" />
          </TabsContent>
          <TabsContent value="node" className="flex-1 min-h-0 pt-3">
            <SampleEditor value={samples.node} language="typescript" />
          </TabsContent>
          <TabsContent value="python" className="flex-1 min-h-0 pt-3">
            <SampleEditor value={samples.python} language="python" />
          </TabsContent>
          <TabsContent value="react" className="flex-1 min-h-0 pt-3">
            <SampleEditor value={samples.react} language="typescript" />
          </TabsContent>
        </Tabs>
      </div>
    </div>
  )
}

const SampleEditor = ({ value, language }: { value: string; language?: string }) => {
  return (
    <div className="h-[65vh] min-h-[240px] overflow-hidden rounded-md border border-slate-800/70 bg-slate-950/80">
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

type CloneArgs = {
  agent: any
  agentId?: string
  definition: AgentDefinitionEnvelope | null
  navigate: ReturnType<typeof useNavigate>
  markdown: string
}

const handleClone = ({ agent, agentId, definition, navigate, markdown }: CloneArgs) => {
  if (!definition) {
    toast.error('Definition not loaded yet')
    return
  }
  const sourceMarkdown = markdown?.trim()
  if (!sourceMarkdown) {
    toast.error('Markdown not loaded yet')
    return
  }
  const baseName = (definition?.name as string) || agent?.name || agentId || 'agent'
  const nextName = getNextCloneName(baseName)
  const cloneMarkdown = updateFrontmatterName(sourceMarkdown, nextName)
  const params = new URLSearchParams({
    name: nextName,
    markdown: cloneMarkdown,
  })
  navigate(`/home/new?${params.toString()}`)
}

const getNextCloneName = (name: string) => {
  const match = name.match(/^(.*?)(?:_clone)?_(\d+)$/)
  if (match) {
    const base = match[1] || name
    const n = Number(match[2]) || 0
    return `${base}_clone_${n + 1}`
  }
  return `${name}_clone_1`
}

const updateFrontmatterName = (markdown: string, name: string): string => {
  const parts = markdown.split('---')
  if (parts.length < 3) return markdown
  const frontmatter = parts[1]
  const line = `${'name'} = ${JSON.stringify(name)}`
  if (frontmatter.includes('name =')) {
    parts[1] = frontmatter.replace(/name\s*=.*$/m, line)
  } else {
    parts[1] = `${frontmatter.trim()}\n${line}\n`
  }
  return parts.join('---')
}
