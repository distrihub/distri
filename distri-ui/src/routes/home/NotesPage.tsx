import { useState, useEffect, useCallback, useMemo } from 'react'
import { BACKEND_URL } from '@/constants'
import { useInitialization } from '@/components/TokenProvider'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Textarea } from '@/components/ui/textarea'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '@/components/ui/dialog'
import {
  Search,
  Plus,
  Trash2,
  ArrowLeft,
  FileText,
  Tag,
  RefreshCw,
  Calendar,
  Loader2,
} from 'lucide-react'
import ReactMarkdown from 'react-markdown'

interface NoteRecord {
  id: string
  user_id: string
  title: string
  content: string
  summary: string
  tags: string[]
  headings: string[]
  keywords: string[]
  created_at: string
  updated_at: string
}

interface NoteListResponse {
  notes: NoteRecord[]
  total: number
}

function useAuthHeaders() {
  const { token } = useInitialization()
  return useMemo(() => {
    const headers: Record<string, string> = { 'Content-Type': 'application/json' }
    if (token) {
      headers['Authorization'] = `Bearer ${token}`
    }
    return headers
  }, [token])
}

export default function NotesPage() {
  const [notes, setNotes] = useState<NoteRecord[]>([])
  const [total, setTotal] = useState(0)
  const [loading, setLoading] = useState(true)
  const [searchQuery, setSearchQuery] = useState('')
  const [selectedNote, setSelectedNote] = useState<NoteRecord | null>(null)
  const [showCreateDialog, setShowCreateDialog] = useState(false)
  const [newTitle, setNewTitle] = useState('')
  const [newContent, setNewContent] = useState('')
  const [newTags, setNewTags] = useState('')
  const [creating, setCreating] = useState(false)
  const [tagFilter, setTagFilter] = useState<string | null>(null)
  const [editMode, setEditMode] = useState(false)
  const [editContent, setEditContent] = useState('')
  const [editTitle, setEditTitle] = useState('')
  const [saving, setSaving] = useState(false)
  const [summarising, setSummarising] = useState(false)
  const headers = useAuthHeaders()

  const fetchNotes = useCallback(async () => {
    setLoading(true)
    try {
      let response: Response

      if (searchQuery.trim() || tagFilter) {
        const body: Record<string, unknown> = {}
        if (searchQuery.trim()) body.query = searchQuery.trim()
        if (tagFilter) body.tags = [tagFilter]

        response = await fetch(`${BACKEND_URL}/v1/notes/search`, {
          method: 'POST',
          headers,
          body: JSON.stringify(body),
        })
      } else {
        response = await fetch(`${BACKEND_URL}/v1/notes?limit=100`, { headers })
      }

      if (response.ok) {
        const data: NoteListResponse = await response.json()
        setNotes(data.notes)
        setTotal(data.total)
      }
    } catch (err) {
      console.error('Failed to fetch notes:', err)
    } finally {
      setLoading(false)
    }
  }, [searchQuery, tagFilter, headers])

  useEffect(() => {
    fetchNotes()
  }, [fetchNotes])

  const handleCreate = async () => {
    if (!newTitle.trim()) return
    setCreating(true)
    try {
      const tags = newTags
        .split(',')
        .map((t) => t.trim())
        .filter(Boolean)
      const response = await fetch(`${BACKEND_URL}/v1/notes`, {
        method: 'POST',
        headers,
        body: JSON.stringify({
          title: newTitle.trim(),
          content: newContent,
          tags,
        }),
      })

      if (response.ok) {
        setShowCreateDialog(false)
        setNewTitle('')
        setNewContent('')
        setNewTags('')
        fetchNotes()
      }
    } catch (err) {
      console.error('Failed to create note:', err)
    } finally {
      setCreating(false)
    }
  }

  const handleDelete = async (id: string) => {
    try {
      const response = await fetch(`${BACKEND_URL}/v1/notes/${id}`, {
        method: 'DELETE',
        headers,
      })
      if (response.ok) {
        if (selectedNote?.id === id) {
          setSelectedNote(null)
        }
        fetchNotes()
      }
    } catch (err) {
      console.error('Failed to delete note:', err)
    }
  }

  const handleSave = async () => {
    if (!selectedNote) return
    setSaving(true)
    try {
      const body: Record<string, unknown> = {}
      if (editTitle !== selectedNote.title) body.title = editTitle
      if (editContent !== selectedNote.content) body.content = editContent

      const response = await fetch(`${BACKEND_URL}/v1/notes/${selectedNote.id}`, {
        method: 'PUT',
        headers,
        body: JSON.stringify(body),
      })
      if (response.ok) {
        const updated: NoteRecord = await response.json()
        setSelectedNote(updated)
        setEditMode(false)
        fetchNotes()
      }
    } catch (err) {
      console.error('Failed to save note:', err)
    } finally {
      setSaving(false)
    }
  }

  const handleSummarise = async (id: string) => {
    setSummarising(true)
    try {
      const response = await fetch(`${BACKEND_URL}/v1/notes/${id}/summarise`, {
        method: 'POST',
        headers,
      })
      if (response.ok) {
        const updated: NoteRecord = await response.json()
        setSelectedNote(updated)
        fetchNotes()
      }
    } catch (err) {
      console.error('Failed to summarise note:', err)
    } finally {
      setSummarising(false)
    }
  }

  // Collect all unique tags across notes
  const allTags = useMemo(() => {
    const tagSet = new Set<string>()
    notes.forEach((n) => n.tags.forEach((t) => tagSet.add(t)))
    return Array.from(tagSet).sort()
  }, [notes])

  const formatDate = (dateStr: string) => {
    const d = new Date(dateStr)
    return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })
  }

  // Detail view for a selected note
  if (selectedNote) {
    return (
      <div className="flex flex-col h-full">
        <div className="flex items-center gap-2 p-4 border-b">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setSelectedNote(null)
              setEditMode(false)
            }}
          >
            <ArrowLeft className="h-4 w-4 mr-1" />
            Back
          </Button>
          <div className="flex-1" />
          {!editMode && (
            <>
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  setEditTitle(selectedNote.title)
                  setEditContent(selectedNote.content)
                  setEditMode(true)
                }}
              >
                Edit
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => handleSummarise(selectedNote.id)}
                disabled={summarising}
              >
                {summarising ? (
                  <Loader2 className="h-3 w-3 animate-spin mr-1" />
                ) : (
                  <RefreshCw className="h-3 w-3 mr-1" />
                )}
                Summarise
              </Button>
              <Button
                variant="destructive"
                size="sm"
                onClick={() => handleDelete(selectedNote.id)}
              >
                <Trash2 className="h-3 w-3 mr-1" />
                Delete
              </Button>
            </>
          )}
          {editMode && (
            <>
              <Button variant="ghost" size="sm" onClick={() => setEditMode(false)}>
                Cancel
              </Button>
              <Button size="sm" onClick={handleSave} disabled={saving}>
                {saving && <Loader2 className="h-3 w-3 animate-spin mr-1" />}
                Save
              </Button>
            </>
          )}
        </div>

        <div className="flex-1 overflow-auto p-6">
          {editMode ? (
            <div className="space-y-4 max-w-4xl">
              <Input
                value={editTitle}
                onChange={(e) => setEditTitle(e.target.value)}
                className="text-xl font-bold"
                placeholder="Note title"
              />
              <Textarea
                value={editContent}
                onChange={(e) => setEditContent(e.target.value)}
                className="min-h-[400px] font-mono text-sm"
                placeholder="Write your markdown note..."
              />
            </div>
          ) : (
            <div className="max-w-4xl">
              <h1 className="text-2xl font-bold mb-2">{selectedNote.title}</h1>

              <div className="flex items-center gap-2 text-sm text-muted-foreground mb-4">
                <Calendar className="h-3 w-3" />
                <span>Created {formatDate(selectedNote.created_at)}</span>
                <span className="mx-1">|</span>
                <span>Updated {formatDate(selectedNote.updated_at)}</span>
              </div>

              {selectedNote.tags.length > 0 && (
                <div className="flex flex-wrap gap-1 mb-4">
                  {selectedNote.tags.map((tag) => (
                    <Badge key={tag} variant="secondary" className="text-xs">
                      <Tag className="h-2.5 w-2.5 mr-1" />
                      {tag}
                    </Badge>
                  ))}
                </div>
              )}

              {selectedNote.summary && (
                <Card className="mb-4">
                  <CardHeader className="pb-2">
                    <CardTitle className="text-sm">Summary</CardTitle>
                  </CardHeader>
                  <CardContent>
                    <p className="text-sm text-muted-foreground">{selectedNote.summary}</p>
                  </CardContent>
                </Card>
              )}

              {selectedNote.headings.length > 0 && (
                <Card className="mb-4">
                  <CardHeader className="pb-2">
                    <CardTitle className="text-sm">Table of Contents</CardTitle>
                  </CardHeader>
                  <CardContent>
                    <ul className="text-sm space-y-1">
                      {selectedNote.headings.map((h, i) => (
                        <li key={i} className="text-muted-foreground">
                          {h}
                        </li>
                      ))}
                    </ul>
                  </CardContent>
                </Card>
              )}

              <div className="prose prose-sm dark:prose-invert max-w-none">
                <ReactMarkdown>{selectedNote.content}</ReactMarkdown>
              </div>

              {selectedNote.keywords.length > 0 && (
                <div className="mt-6 pt-4 border-t">
                  <p className="text-xs text-muted-foreground mb-2">Keywords</p>
                  <div className="flex flex-wrap gap-1">
                    {selectedNote.keywords.map((kw) => (
                      <Badge key={kw} variant="outline" className="text-xs">
                        {kw}
                      </Badge>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    )
  }

  // List view
  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-3 p-4 border-b">
        <div className="flex items-center gap-2">
          <FileText className="h-5 w-5" />
          <h2 className="text-lg font-semibold">Notes</h2>
          <Badge variant="secondary" className="text-xs">
            {total}
          </Badge>
        </div>
        <div className="flex-1" />
        <div className="relative w-64">
          <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            placeholder="Search notes..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="pl-8"
          />
        </div>
        <Button size="sm" onClick={() => setShowCreateDialog(true)}>
          <Plus className="h-4 w-4 mr-1" />
          New Note
        </Button>
      </div>

      {/* Tag filters */}
      {allTags.length > 0 && (
        <div className="flex flex-wrap gap-1 px-4 py-2 border-b">
          <Button
            variant={tagFilter === null ? 'default' : 'outline'}
            size="sm"
            className="h-6 text-xs"
            onClick={() => setTagFilter(null)}
          >
            All
          </Button>
          {allTags.map((tag) => (
            <Button
              key={tag}
              variant={tagFilter === tag ? 'default' : 'outline'}
              size="sm"
              className="h-6 text-xs"
              onClick={() => setTagFilter(tagFilter === tag ? null : tag)}
            >
              {tag}
            </Button>
          ))}
        </div>
      )}

      {/* Notes grid */}
      <div className="flex-1 overflow-auto p-4">
        {loading ? (
          <div className="flex items-center justify-center h-32">
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
        ) : notes.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-32 text-muted-foreground">
            <FileText className="h-8 w-8 mb-2" />
            <p>{searchQuery || tagFilter ? 'No notes match your search' : 'No notes yet'}</p>
          </div>
        ) : (
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {notes.map((note) => (
              <Card
                key={note.id}
                className="cursor-pointer hover:border-primary/50 transition-colors"
                onClick={() => setSelectedNote(note)}
              >
                <CardHeader className="pb-2">
                  <CardTitle className="text-sm truncate">{note.title}</CardTitle>
                  <CardDescription className="text-xs">
                    {formatDate(note.updated_at)}
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  {note.summary ? (
                    <p className="text-xs text-muted-foreground line-clamp-2">{note.summary}</p>
                  ) : (
                    <p className="text-xs text-muted-foreground line-clamp-2">
                      {note.content.slice(0, 150)}
                    </p>
                  )}
                  {note.tags.length > 0 && (
                    <div className="flex flex-wrap gap-1 mt-2">
                      {note.tags.slice(0, 3).map((tag) => (
                        <Badge key={tag} variant="secondary" className="text-[10px] px-1.5 py-0">
                          {tag}
                        </Badge>
                      ))}
                      {note.tags.length > 3 && (
                        <Badge variant="outline" className="text-[10px] px-1.5 py-0">
                          +{note.tags.length - 3}
                        </Badge>
                      )}
                    </div>
                  )}
                </CardContent>
              </Card>
            ))}
          </div>
        )}
      </div>

      {/* Create Note Dialog */}
      <Dialog open={showCreateDialog} onOpenChange={setShowCreateDialog}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>Create Note</DialogTitle>
          </DialogHeader>
          <div className="space-y-4">
            <Input
              placeholder="Note title"
              value={newTitle}
              onChange={(e) => setNewTitle(e.target.value)}
            />
            <Textarea
              placeholder="Write your markdown note content..."
              value={newContent}
              onChange={(e) => setNewContent(e.target.value)}
              className="min-h-[250px] font-mono text-sm"
            />
            <Input
              placeholder="Tags (comma-separated, e.g. meeting-notes, project-x)"
              value={newTags}
              onChange={(e) => setNewTags(e.target.value)}
            />
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setShowCreateDialog(false)}>
              Cancel
            </Button>
            <Button onClick={handleCreate} disabled={!newTitle.trim() || creating}>
              {creating && <Loader2 className="h-3 w-3 animate-spin mr-1" />}
              Create
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
