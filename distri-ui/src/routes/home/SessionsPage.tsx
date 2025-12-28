import { useEffect, useMemo, useState } from 'react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { useHomeFetch } from '@/hooks/useHomeFetch'

interface SessionListItem {
  session_id: string
  thread_id: string
  key_count: number
  keys: string[]
  updated_at?: string | null
  task_ids: string[]
}

const SessionsPage = () => {
  const homeFetch = useHomeFetch()
  const [sessions, setSessions] = useState<SessionListItem[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [threadFilter, setThreadFilter] = useState('')
  const [taskFilter, setTaskFilter] = useState('')
  const [keyFilter, setKeyFilter] = useState('')

  const loadSessions = async () => {
    setLoading(true)
    setError(null)
    try {
      const res = await homeFetch('/api/v1/session')
      if (!res.ok) {
        throw new Error(`Status ${res.status}`)
      }
      const payload = (await res.json()) as SessionListItem[]
      setSessions(payload)
    } catch (err: any) {
      setError(err?.message || 'Failed to load sessions')
      setSessions([])
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    loadSessions()
  }, [homeFetch])

  const filtered = useMemo(() => {
    return sessions.filter((session) => {
      const matchesThread = threadFilter
        ? session.thread_id.toLowerCase().includes(threadFilter.toLowerCase())
        : true
      const matchesTask = taskFilter
        ? session.task_ids.some((task) => task.toLowerCase().includes(taskFilter.toLowerCase()))
        : true
      const matchesKeys = keyFilter
        ? session.keys.some((key) => key.toLowerCase().includes(keyFilter.toLowerCase()))
        : true
      return matchesThread && matchesTask && matchesKeys
    })
  }, [sessions, threadFilter, taskFilter, keyFilter])

  return (
    <div className="flex-1 overflow-auto">
      <div className="mx-auto w-full max-w-none px-4 py-6 sm:px-6 lg:px-8 lg:py-10">
        <Card className="border-border/70 bg-card/95">
          <CardHeader className="flex flex-col gap-3 pb-3 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <CardTitle className="text-base">Sessions</CardTitle>
              <p className="text-sm text-muted-foreground">
                Browse session storage and related task runs.
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={loadSessions}>
              Refresh
            </Button>
          </CardHeader>
          <CardContent className="space-y-4 pt-0">
            <div className="grid gap-2 sm:grid-cols-3">
              <Input
                placeholder="Filter by thread ID…"
                value={threadFilter}
                onChange={(e) => setThreadFilter(e.target.value)}
              />
              <Input
                placeholder="Filter by task ID…"
                value={taskFilter}
                onChange={(e) => setTaskFilter(e.target.value)}
              />
              <Input
                placeholder="Filter by key…"
                value={keyFilter}
                onChange={(e) => setKeyFilter(e.target.value)}
              />
            </div>

            {error && (
              <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                {error}
              </div>
            )}

            <div className="overflow-x-auto rounded-md border">
              <table className="w-full text-left text-sm">
                <thead className="bg-muted/50">
                  <tr>
                    <th className="px-3 py-2 font-medium">Session</th>
                    <th className="px-3 py-2 font-medium">Thread</th>
                    <th className="px-3 py-2 font-medium">Tasks</th>
                    <th className="px-3 py-2 font-medium">Keys</th>
                    <th className="px-3 py-2 font-medium">Updated</th>
                  </tr>
                </thead>
                <tbody>
                  {loading ? (
                    <tr>
                      <td className="px-3 py-3 text-muted-foreground" colSpan={5}>
                        Loading…
                      </td>
                    </tr>
                  ) : filtered.length === 0 ? (
                    <tr>
                      <td className="px-3 py-3 text-muted-foreground" colSpan={5}>
                        No sessions found.
                      </td>
                    </tr>
                  ) : (
                    filtered.map((session) => {
                      const visibleKeys = session.keys.slice(0, 3)
                      const remaining = session.key_count - visibleKeys.length
                      return (
                        <tr key={session.session_id} className="border-t">
                          <td className="px-3 py-2 max-w-[20rem]">
                            <div className="truncate font-medium">{session.session_id}</div>
                          </td>
                          <td className="px-3 py-2 max-w-[20rem]">
                            <div className="truncate">{session.thread_id}</div>
                          </td>
                          <td className="px-3 py-2">
                            <div className="flex flex-wrap gap-1">
                              {session.task_ids.length === 0 ? (
                                <span className="text-muted-foreground">—</span>
                              ) : (
                                session.task_ids.slice(0, 2).map((taskId) => (
                                  <Badge key={taskId} variant="secondary" className="font-mono text-xs">
                                    {taskId}
                                  </Badge>
                                ))
                              )}
                              {session.task_ids.length > 2 && (
                                <Badge variant="outline" className="text-xs">
                                  +{session.task_ids.length - 2}
                                </Badge>
                              )}
                            </div>
                          </td>
                          <td className="px-3 py-2">
                            <div className="flex flex-wrap gap-1">
                              {visibleKeys.map((key) => (
                                <Badge key={key} variant="outline" className="text-xs">
                                  {key}
                                </Badge>
                              ))}
                              {remaining > 0 && (
                                <span className="text-xs text-muted-foreground">+{remaining} more</span>
                              )}
                            </div>
                          </td>
                          <td className="px-3 py-2">
                            {session.updated_at ? new Date(session.updated_at).toLocaleString() : '—'}
                          </td>
                        </tr>
                      )
                    })
                  )}
                </tbody>
              </table>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  )
}

export default SessionsPage
