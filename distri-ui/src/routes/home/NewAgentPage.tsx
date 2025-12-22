import { useEffect, useMemo, useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
import Editor from '@monaco-editor/react'
import { ChatInput, useAgentDefinitions } from '@distri/react'
import type { DistriPart } from '@distri/core'
import { v4 as uuidv4 } from 'uuid'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Loader2 } from 'lucide-react'
import { BACKEND_URL } from '@/constants'
import { toast } from 'sonner'
import { useInitialization } from '@/components/TokenProvider'

const NewAgentPage = () => {
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const initialCloneMarkdown = searchParams.get('markdown') || ''
  const initialCloneName = searchParams.get('name') || ''
  const { token } = useInitialization()
  const { agents, loading } = useAgentDefinitions()
  const [prompt, setPrompt] = useState('')
  const [markdown, setMarkdown] = useState(initialCloneMarkdown)
  const [name, setName] = useState(initialCloneName)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    setMarkdown(initialCloneMarkdown)
    setName(initialCloneName)
  }, [initialCloneMarkdown, initialCloneName])

  const preferredAgent = useMemo(() => {
    if (!agents.length) {
      return undefined
    }
    return (
      agents.find((agent) => agent.id === 'scripter' || agent.name?.toLowerCase() === 'scripter') ||
      agents[0]
    )
  }, [agents])

  const handleSend = (content: string | DistriPart[]) => {
    const text = typeof content === 'string'
      ? content
      : content.map((part) => (part.part_type === 'text' ? part.data : '')).join('\n')

    if (!text.trim() || !preferredAgent) {
      return
    }

    const params = new URLSearchParams()
    params.set('prefill', text.trim())
    params.set('threadId', uuidv4())
    navigate(`/home/agents/${encodeURIComponent(preferredAgent.id)}?${params.toString()}`)
    setPrompt('')
  }

  const handleCreateClone = async () => {
    if (!markdown.trim() || !name.trim()) {
      toast.error('Name and markdown are required')
      return
    }
    const finalMarkdown = updateFrontmatterValue(markdown, 'name', name.trim())
    setSaving(true)
    try {
      const headers: Record<string, string> = { 'Content-Type': 'text/plain' }
      if (token) {
        headers['Authorization'] = `Bearer ${token}`
      }
      const resp = await fetch(`${BACKEND_URL}/api/v1/agents`, {
        method: 'POST',
        headers,
        body: finalMarkdown,
      })
      if (!resp.ok) {
        const message = await resp.text()
        throw new Error(message || 'Failed to create agent')
      }
      const data = await resp.json()
      const newId = data.id || data.agent_id || data.name || name
      toast.success('Agent cloned')
      navigate(`/home/agents/${encodeURIComponent(newId)}`)
    } catch (err) {
      console.error(err)
      toast.error(err instanceof Error ? err.message : 'Failed to create agent')
    } finally {
      setSaving(false)
    }
  }

  if (initialCloneMarkdown) {
    return (
      <div className="flex h-full w-full justify-center">
        <div className="w-full max-w-6xl space-y-4 p-4 sm:p-6 lg:p-8">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-xs uppercase tracking-[0.35em] text-muted-foreground">Clone agent</p>
              <p className="text-sm text-muted-foreground">Review and save the cloned definition.</p>
            </div>
            <div className="flex items-center gap-2">
              <Button variant="outline" onClick={() => navigate(-1)}>
                Cancel
              </Button>
              <Button
                onClick={handleCreateClone}
                disabled={saving || !name.trim() || !markdown.trim()}
                className="gap-2"
              >
                {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                Create
              </Button>
            </div>
          </div>

          <div className="space-y-2">
            <label className="text-xs uppercase tracking-[0.25em] text-muted-foreground">Name</label>
            <Input
              value={name}
              onChange={(e) => {
                const next = e.target.value
                setName(next)
                setMarkdown((prev) => updateFrontmatterValue(prev, 'name', next))
              }}
              placeholder="agent_clone_1"
            />
          </div>

          <div className="h-[70vh] min-h-[360px] overflow-hidden rounded-lg border border-border/60 bg-card/80">
            <Editor
              height="100%"
              value={markdown}
              defaultLanguage="markdown"
              theme="vs-dark"
              onChange={(value) => setMarkdown(value ?? '')}
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

  return (
    <div className="flex h-full w-full items-center justify-center ">
      <div className="w-full max-w-2xl space-y-6 p-4 shadow-sm sm:p-6 lg:p-8">
        <div className="space-y-2">
          <p className="text-xs uppercase tracking-[0.35em] text-muted-foreground">New agent</p>
        </div>
        <ChatInput
          value={prompt}
          onChange={setPrompt}
          onSend={handleSend}
          placeholder={'Describe the agent you want to build'}
          disabled={!preferredAgent || loading}
          variant="hero"
        />
      </div>
    </div>
  )
}

export default NewAgentPage

const updateFrontmatterValue = (markdown: string, key: string, value: string): string => {
  const parts = markdown.split('---')
  if (parts.length < 3) return markdown
  const frontmatter = parts[1]
  const line = `${key} = ${JSON.stringify(value)}`
  if (frontmatter.match(new RegExp(`^\\s*${key}\\s*=`, 'm'))) {
    parts[1] = frontmatter.replace(new RegExp(`^\\s*${key}\\s*=.*$`, 'm'), line)
  } else {
    parts[1] = `${frontmatter.trim()}\n${line}\n`
  }
  return parts.join('---')
}
