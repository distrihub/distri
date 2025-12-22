import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import type { FileSaveHandler, FileWorkspaceStore } from '@distri/fs'
import {
  FileWorkspace,
  IndexedDbFilesystem,
  createFileWorkspaceStore,
  ScriptTestingPanel,
  createFilesystemTools,
  ScriptRunnerTool,
  type FileActionItem,
  type FileActionRenderer,
} from '@distri/fs'
import { useAgent, Chat, type DistriAnyTool } from '@distri/react'
import { BACKEND_URL } from '@/constants'
import { useInitialization } from '@/components/TokenProvider'
import { Loader2, Home, Palette, Play } from 'lucide-react'
import { toast } from 'sonner'
import { uuidv4 } from '@distri/core'
interface WorkspaceMetadataEntry {
  path: string
  is_dir: boolean
  size: number
  modified: string
}

interface WorkspaceMetadataResponse {
  files: WorkspaceMetadataEntry[]
  updated_at: string
}

const API_BASE_URL = `${BACKEND_URL}/api/v1`
const PROJECT_ID = 'distri-workspace'

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

const encodeWorkspacePath = (path: string) =>
  path
    .split('/')
    .filter(Boolean)
    .map((segment) => encodeURIComponent(segment))
    .join('/')

const FilesPage = () => {
  const { token } = useInitialization()
  const { agent, loading: agentLoading, error: agentError } = useAgent({ agentIdOrDef: 'agent_designer' })

  const filesystem = useMemo(() => IndexedDbFilesystem.forProject(PROJECT_ID), [])
  const storeRef = useRef<FileWorkspaceStore | null>(null)
  if (!storeRef.current) {
    storeRef.current = createFileWorkspaceStore(PROJECT_ID, { filesystem })
  }
  const store = storeRef.current

  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [workspaceReady, setWorkspaceReady] = useState(false)
  const threadId = useMemo(() => currentThreadId(PROJECT_ID), [])

  const authHeaders = useMemo(() => {
    if (!token) return undefined
    return { Authorization: `Bearer ${token}` }
  }, [token])

  const applyMetadata = useCallback(
    async (entries: WorkspaceMetadataEntry[]) => {
      const sorted = [...entries].sort((a, b) => a.path.split('/').length - b.path.split('/').length)
      for (const entry of sorted) {
        const timestamp = entry.modified ? Date.parse(entry.modified) : Date.now()
        if (entry.is_dir) {
          await filesystem.upsertMetadata(entry.path, 'directory', timestamp).catch(() => { })
        } else {
          await filesystem.upsertMetadata(entry.path, 'file', timestamp)
        }
      }
      await store.getState().refresh()
      setWorkspaceReady(true)
    },
    [filesystem, store],
  )

  const fetchMetadata = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const response = await fetch(`${API_BASE_URL}/files/metadata`, { headers: authHeaders })
      if (!response.ok) {
        const message = await response.text()
        throw new Error(message || 'Failed to load workspace files')
      }
      const snapshot: WorkspaceMetadataResponse = await response.json()
      await applyMetadata(snapshot.files)
    } catch (err) {
      console.error(err)
      setError(err instanceof Error ? err.message : 'Failed to load workspace files')
      toast.error(err instanceof Error ? err.message : 'Failed to load workspace files')
    } finally {
      setLoading(false)
    }
  }, [applyMetadata, authHeaders])

  useEffect(() => {
    const hydrateFromCache = async () => {
      try {
        await store.getState().refresh()
        setWorkspaceReady(true)
      } catch (err) {
        console.error('Failed to hydrate local workspace cache', err)
      }
    }
    void hydrateFromCache()
  }, [store])

  useEffect(() => {
    if (!workspaceReady) {
      return
    }
    void fetchMetadata()
  }, [workspaceReady, fetchMetadata])

  useEffect(() => {
    filesystem.setRemoteFetcher(async (path, currentVersion) => {
      const encoded = encodeWorkspacePath(path)
      const headers: Record<string, string> = { ...(authHeaders ?? {}) }
      if (currentVersion) {
        headers['If-Modified-Since'] = new Date(currentVersion).toUTCString()
      }
      const resp = await fetch(`${API_BASE_URL}/files/${encoded}`, { headers })
      if (resp.status === 304) {
        return null
      }
      if (resp.status === 404) {
        return { content: '', updatedAt: Date.now() }
      }
      if (!resp.ok) {
        const message = await resp.text()
        throw new Error(message || `Failed to load workspace file ${path}`)
      }
      const data = await resp.json()
      return {
        content: data.content ?? '',
        updatedAt: data.updated_at ? Date.parse(data.updated_at) : Date.now(),
      }
    })
  }, [authHeaders, filesystem])

  const handleSaveFile: FileSaveHandler = useCallback(async (tab) => {
    const response = await fetch(`${API_BASE_URL}/files`, {
      method: 'PUT',
      headers: {
        'Content-Type': 'application/json',
        ...(authHeaders ?? {}),
      },
      body: JSON.stringify({ files: [{ path: tab.path, content: tab.content }] }),
    })
    if (!response.ok) {
      const message = await response.text()
      throw new Error(message || `Failed to save ${tab.path}`)
    }

    toast.success('Saved', { description: tab.path, duration: 1500 })
  }, [authHeaders])

  const isReady = workspaceReady

  // Build filesystem tools for chat (so the coder agent can operate on the workspace)
  const filesystemTools = useMemo(() => (
    createFilesystemTools(PROJECT_ID, {
      filesystem,
      onChange: (event) => {
        void store.getState().handleExternalChange(event)
      },
    })
  ), [filesystem, store])

  const uiTools = useMemo<DistriAnyTool[]>(() => [ScriptRunnerTool], [])
  const externalTools = useMemo(() => [...filesystemTools, ...uiTools], [filesystemTools, uiTools])
  const navigate = useNavigate();

  const activityBarItems = useMemo(() => (
    [
      {
        id: 'workspace-home',
        label: 'Home',
        icon: Home,
        position: 'left' as const,
        type: 'action' as const,
        order: -100,
        onSelect: () => navigate('/home'),
      },
      {
        id: 'designer-chat',
        label: 'Preview',
        icon: Palette,
        position: 'right' as const,
        mode: 'custom' as const,
        content: (
          <div className="flex h-full flex-col gap-3">
            <ChatSidebarPanel
              agent={agent}
              agentLoading={agentLoading}
              threadId={threadId}
              externalTools={externalTools}
            />
          </div>
        ),
      },
    ]
  ), [agent, agentLoading, externalTools, navigate, threadId])


  const fileActionItems = useMemo<FileActionItem[]>(() => (
    [
      {
        id: 'run-script',
        label: 'Run script',
        icon: Play,
        isVisible: ({ tab }) => tab.path.includes('/scripts/') || tab.path.endsWith('.script.ts'),
        onSelect: ({ tab }) => {
          toast.info('Running script', { description: tab.path })
        },
      },
    ]
  ), [])

  const fileActionRenderers = useMemo<FileActionRenderer[]>(() => (
    [
      {
        id: 'ts-testing',
        label: 'Script testing',
        match: ({ tab }) => tab.path.endsWith('.ts') || tab.path.endsWith('.tsx'),
        render: ({ tab }) => (
          <ScriptTestingPanel
            key={tab.path}
            title={`Test ${tab.path.split('/').pop()}`}
            description="Send sample payloads to the coder agent."
            resultPlaceholder="Run a test to see agent output."
            onRun={async () => 'Connect your script runner to execute tests.'}
          />
        ),
      },
    ]
  ), [])




  return (
    <div className="flex h-full w-full flex-col bg-background text-foreground">
      {error ? (
        <div className="border-b border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      ) : null}

      {agentError ? (
        <div className="border-b border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-600 dark:text-amber-200">
          Unable to load the Coder agent. Ensure it is registered before using the workspace chat.
        </div>
      ) : null}

      <div className="flex-1 min-h-0 bg-card">
        {isReady ? (
          <FileWorkspace
            projectId={PROJECT_ID}
            filesystem={filesystem}
            store={store}
            onSaveFile={handleSaveFile}
            onSyncWorkspace={() => void fetchMetadata()}
            isSyncingWorkspace={loading}
            className="h-full"
            activityBarItems={activityBarItems}
            defaultActivityId="explorer"
            fileActionItems={fileActionItems}
            fileActionRenderers={fileActionRenderers}
          />
        ) : (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            <Loader2 className="mr-2 h-5 w-5 animate-spin" />
            Preparing workspace…
          </div>
        )}
      </div>
    </div>
  )
}

interface ChatSidebarPanelProps {
  agent: ReturnType<typeof useAgent>['agent']
  agentLoading: boolean
  threadId: string
  externalTools: DistriAnyTool[]
}

const ChatSidebarPanel = ({ agent, agentLoading, threadId, externalTools }: ChatSidebarPanelProps) => {
  if (!agent) {
    return (
      <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
        {agentLoading ? 'Connecting to agent…' : 'Agent unavailable'}
      </div>
    )
  }

  return (
    <div className="flex h-full flex-col gap-3">
      <div className="flex-1 overflow-hidden">
        <Chat agent={agent} threadId={threadId} externalTools={externalTools} theme="auto" />
      </div>
    </div>
  )
}


export default FilesPage
