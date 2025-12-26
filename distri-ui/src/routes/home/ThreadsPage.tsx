import { useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { useThreads } from '@distri/react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'

const ThreadsPage = () => {
  const { threads, loading } = useThreads()
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const agentQuery = searchParams.get('agent') || searchParams.get('agent_id') || ''
  const [agentFilter, setAgentFilter] = useState(agentQuery)
  const [userFilter, setUserFilter] = useState('')
  const [query, setQuery] = useState('')
  const [visibleCount, setVisibleCount] = useState(20)
  const sentinelRef = useRef<HTMLDivElement | null>(null)

  const agentOptions = useMemo(() => {
    const set = new Set<string>()
    threads.forEach((t: any) => {
      if (t.agent_name) set.add(t.agent_name)
    })
    return Array.from(set).sort((a, b) => a.localeCompare(b))
  }, [threads])

  useEffect(() => {
    setAgentFilter(agentQuery)
  }, [agentQuery])

  useEffect(() => {
    // Auto-load more when reaching bottom
    const node = sentinelRef.current
    if (!node) return
    const obs = new IntersectionObserver((entries) => {
      for (const e of entries) {
        if (e.isIntersecting) {
          setVisibleCount((n) => n + 20)
        }
      }
    }, { root: null, rootMargin: '0px', threshold: 1 })
    obs.observe(node)
    return () => obs.disconnect()
  }, [])

  const filtered = useMemo(() => {
    return threads.filter((t: any) => {
      const agentLabel = `${t.agent_name || ''} ${t.agent_id || ''}`.trim()
      const matchesAgent = agentFilter
        ? agentLabel.toLowerCase().includes(agentFilter.toLowerCase())
        : true
      const userName = (t.user_name || t.user || '').toString()
      const matchesUser = userFilter ? userName.toLowerCase().includes(userFilter.toLowerCase()) : true
      const matchesQuery =
        query
          ? ((t.title || '') + ' ' + (t.last_message || '')).toLowerCase().includes(query.toLowerCase())
          : true
      return matchesAgent && matchesUser && matchesQuery
    })
  }, [threads, agentFilter, userFilter, query])

  return (
    <div className="flex-1 overflow-auto">
      <div className="mx-auto w-full max-w-none px-4 py-6 sm:px-6 lg:px-8 lg:py-10">
        <Card className="border-border/70 bg-card/95">
          <CardHeader className="pb-3">
            <CardTitle className="text-base">Threads</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4 pt-0">
            <div className="grid gap-2 sm:grid-cols-3">
              <Input
                placeholder="Filter by agent…"
                value={agentFilter}
                onChange={(e) => setAgentFilter(e.target.value)}
                list="agent-options"
              />
              <datalist id="agent-options">
                {agentOptions.map((a) => (
                  <option key={a} value={a} />
                ))}
              </datalist>
              <Input
                placeholder="Filter by user…"
                value={userFilter}
                onChange={(e) => setUserFilter(e.target.value)}
              />
              <Input
                placeholder="Search in titles/messages…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>

            <div className="overflow-x-auto rounded-md border">
              <table className="w-full text-left text-sm">
                <thead className="bg-muted/50">
                  <tr>
                    <th className="px-3 py-2 font-medium">Title</th>
                    <th className="px-3 py-2 font-medium">Agent</th>
                    <th className="px-3 py-2 font-medium">User</th>
                    <th className="px-3 py-2 font-medium">Updated</th>
                    <th className="px-3 py-2 font-medium">Messages</th>
                    <th className="px-3 py-2 font-medium"></th>
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
                        No threads found.
                      </td>
                    </tr>
                  ) : (
                    filtered.slice(0, visibleCount).map((t: any) => (
                      <tr key={t.id} className="border-t">
                        <td className="px-3 py-2 max-w-[28rem]">
                          <div className="truncate font-medium">{t.title || 'Untitled thread'}</div>
                          <div className="text-xs text-muted-foreground truncate">{t.id}</div>
                        </td>
                        <td className="px-3 py-2">{t.agent_name || '—'}</td>
                        <td className="px-3 py-2">{t.user_name || t.user || '—'}</td>
                        <td className="px-3 py-2">{t.updated_at ? new Date(t.updated_at).toLocaleString() : '—'}</td>
                        <td className="px-3 py-2">{t.message_count ?? '—'}</td>
                        <td className="px-3 py-2">
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => {
                              if (t.agent_id) {
                                navigate(`/home/chat?id=${encodeURIComponent(t.agent_id)}&threadId=${t.id}`)
                              }
                            }}
                          >
                            Open
                          </Button>
                        </td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
            {/* Pagination controls */}
            {!loading && filtered.length > visibleCount && (
              <div className="flex items-center justify-center">
                <Button variant="outline" onClick={() => setVisibleCount((n) => n + 20)}>
                  Load more
                </Button>
              </div>
            )}
            {/* Sentinel for auto-load (invisible) */}
            <div ref={sentinelRef} />
          </CardContent>
        </Card>
      </div>
    </div>
  )
}

export default ThreadsPage

