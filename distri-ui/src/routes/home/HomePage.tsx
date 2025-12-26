import { useMemo } from 'react'
import { useNavigate } from 'react-router-dom'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { useAgentDefinitions, useThreads } from '@distri/react'
import { MessageSquare, Users, Gauge } from 'lucide-react'
import { useHomeStats } from '@/hooks/useHomeStats'

const HomePage = () => {
  const navigate = useNavigate()
  const { agents, loading: agentsLoading } = useAgentDefinitions()
  const { threads, loading: threadsLoading } = useThreads()
  const { stats, loading: statsLoading } = useHomeStats()

  const latestThreads = useMemo(() => {
    const sorted = [...threads].sort((a: any, b: any) => {
      const aDate = new Date(a.updated_at || 0).getTime()
      const bDate = new Date(b.updated_at || 0).getTime()
      return bDate - aDate
    })
    return sorted.slice(0, 5)
  }, [threads])

  return (
    <div className="flex-1 overflow-auto">
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 px-4 py-6 sm:px-6 lg:px-8 lg:py-10">
        <div className="grid gap-4 lg:grid-cols-3">
          {/* Metrics card on left (2 columns) */}
          <Card className="border-border/70 bg-card/95 lg:col-span-2">
            <CardHeader className="flex-row items-center justify-between pb-2">
              <CardTitle className="text-base flex items-center gap-2">
                <Gauge className="h-4 w-4" />
                Overview
              </CardTitle>
            </CardHeader>
            <CardContent>
              <div className="grid grid-cols-2 gap-4 md:grid-cols-3">
                <div>
                  <div className="text-xs text-muted-foreground">Total messages</div>
                  <div className="text-2xl font-semibold">
                    {statsLoading ? '—' : (stats?.total_messages ?? '—')}
                  </div>
                </div>
                <div>
                  <div className="text-xs text-muted-foreground">Total threads</div>
                  <div className="text-2xl font-semibold">
                    {statsLoading ? '—' : (stats?.total_threads ?? '—')}
                  </div>
                </div>
                <div>
                  <div className="text-xs text-muted-foreground">Avg time per run</div>
                  <div className="text-2xl font-semibold">
                    {statsLoading
                      ? '—'
                      : (stats?.avg_time_per_run_ms != null
                        ? `${Math.round((stats.avg_time_per_run_ms / 1000) * 10) / 10}s`
                        : '—')}
                  </div>
                </div>
              </div>
            </CardContent>
          </Card>
          {/* Big Agents box on right */}
          <Card className="border-border/70 bg-card/95">
            <CardHeader className="flex-row items-center justify-between pb-2">
              <CardTitle className="text-base flex items-center gap-2">
                <Users className="h-4 w-4" />
                Agents
              </CardTitle>
              <Button size="sm" variant="outline" onClick={() => navigate('/home/agents')}>
                View All
              </Button>
            </CardHeader>
            <CardContent>
              <div className="text-5xl font-semibold">{agentsLoading ? '—' : agents.length}</div>
            </CardContent>
          </Card>
        </div>

        {/* Latest Threads below, limited to 5 */}
        <Card className="border-border/70 bg-card/95">
          <CardHeader className="flex-row items-center justify-between pb-2">
            <CardTitle className="text-base flex items-center gap-2">
              <MessageSquare className="h-4 w-4" />
              Latest Threads
            </CardTitle>
            <Button size="sm" variant="outline" onClick={() => navigate('/home/threads')}>
              View All
            </Button>
          </CardHeader>
          <CardContent className="space-y-3">
            {threadsLoading ? (
              <div className="text-sm text-muted-foreground">Loading…</div>
            ) : latestThreads.length === 0 ? (
              <div className="text-sm text-muted-foreground">No conversations yet.</div>
            ) : (
              latestThreads.slice(0, 5).map((t: any) => (
                <div key={t.id} className="flex items-center justify-between gap-3">
                  <div className="min-w-0">
                    <div className="text-sm font-medium truncate">{t.title || 'Untitled thread'}</div>
                    <div className="text-xs text-muted-foreground truncate">{t.agent_name}</div>
                  </div>
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
                </div>
              ))
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  )
}

export default HomePage


