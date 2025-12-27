import { useEffect, useMemo, useRef, useState } from 'react';
import { useThreads } from '@distri/react';
import { useDistriHomeNavigate } from '../DistriHomeProvider';
import { AlertTriangle, ArrowRight, Filter, Search } from 'lucide-react';

// Thread type - matches what useThreads returns
interface Thread {
  id: string;
  title?: string;
  agent_name?: string;
  agent_id?: string;
  user_name?: string;
  user?: string;
  last_message?: string;
  updated_at?: string;
  message_count?: number;
}

export interface ThreadsViewProps {
  /**
   * Optional custom class name
   */
  className?: string;
}

export function ThreadsView({ className }: ThreadsViewProps) {
  const { threads: rawThreads, loading, error } = useThreads();
  const navigate = useDistriHomeNavigate();
  const [agentFilter, setAgentFilter] = useState('');
  const [userFilter, setUserFilter] = useState('');
  const [query, setQuery] = useState('');
  const [showFilters, setShowFilters] = useState(false);
  const [visibleCount, setVisibleCount] = useState(20);
  const sentinelRef = useRef<HTMLDivElement | null>(null);

  // Cast to our Thread type
  const threads = rawThreads as unknown as Thread[];

  const agentOptions = useMemo(() => {
    const set = new Set<string>();
    threads.forEach((t) => {
      if (t.agent_name) set.add(t.agent_name);
    });
    return Array.from(set).sort((a, b) => a.localeCompare(b));
  }, [threads]);

  useEffect(() => {
    const node = sentinelRef.current;
    if (!node) return;
    const obs = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) {
            setVisibleCount((n) => n + 20);
          }
        }
      },
      { root: null, rootMargin: '0px', threshold: 1 }
    );
    obs.observe(node);
    return () => obs.disconnect();
  }, []);

  const filtered = useMemo(() => {
    return threads.filter((t) => {
      const matchesAgent = agentFilter
        ? (t.agent_name || '').toLowerCase().includes(agentFilter.toLowerCase())
        : true;
      const userName = (t.user_name || t.user || '').toString();
      const matchesUser = userFilter
        ? userName.toLowerCase().includes(userFilter.toLowerCase())
        : true;
      const matchesQuery = query
        ? ((t.title || '') + ' ' + (t.last_message || ''))
          .toLowerCase()
          .includes(query.toLowerCase())
        : true;
      return matchesAgent && matchesUser && matchesQuery;
    });
  }, [threads, agentFilter, userFilter, query]);

  const totalMessages = useMemo(() => {
    return threads.reduce((sum, thread) => sum + (thread.message_count || 0), 0);
  }, [threads]);

  const uniqueAgents = useMemo(() => {
    const set = new Set<string>();
    threads.forEach((t) => {
      if (t.agent_name) set.add(t.agent_name);
    });
    return set.size;
  }, [threads]);

  const latestActivity = useMemo(() => {
    if (error) return 'Unavailable';
    if (!threads.length) return '—';
    const newest = [...threads].sort((a, b) => {
      const aDate = new Date(a.updated_at || 0).getTime();
      const bDate = new Date(b.updated_at || 0).getTime();
      return bDate - aDate;
    })[0];
    return formatRelativeTime(newest?.updated_at);
  }, [threads, error]);

  const showWarning = Boolean(error);
  const threadsCountValue = loading || error ? '—' : formatNumber(threads.length);
  const messageCountValue = loading || error ? '—' : formatNumber(totalMessages);
  const uniqueAgentsValue = loading || error ? '—' : formatNumber(uniqueAgents);

  return (
    <div className={`flex-1 overflow-hidden ${className ?? ''}`}>
      <div className="flex h-full flex-col bg-background">
        <header className="flex h-16 items-center justify-between border-b border-border/60 px-6 lg:px-10">
          <h1 className="text-xl font-semibold text-foreground">Threads</h1>
          <div className="flex items-center gap-3">
            <div className="relative hidden sm:block">
              <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search threads..."
                className="h-9 w-64 rounded-md border border-border/70 bg-card pl-9 pr-3 text-sm text-foreground shadow-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
              />
            </div>
            <button
              type="button"
              onClick={() => setShowFilters((open) => !open)}
              className="inline-flex items-center gap-2 rounded-md border border-border/70 bg-card px-3 py-2 text-sm font-medium text-muted-foreground transition hover:text-primary"
            >
              <Filter className="h-4 w-4" />
              Filters
            </button>
          </div>
        </header>

        <div className="flex-1 overflow-y-auto px-6 py-6 lg:px-10">
          {showWarning ? (
            <div className="mb-6 flex items-start gap-3 rounded-lg border border-amber-300/50 bg-amber-100/60 px-4 py-3 text-xs text-amber-900 dark:border-amber-400/20 dark:bg-amber-400/10 dark:text-amber-100">
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
              <div>
                <p className="font-semibold">We couldn't load threads right now.</p>
                <p className="mt-1 text-amber-800/90 dark:text-amber-100/90">
                  Some stats may be unavailable. Try refreshing soon.
                </p>
              </div>
            </div>
          ) : null}

          <div className="mb-5 block sm:hidden">
            <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
              Search
            </label>
            <div className="relative mt-2">
              <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search threads..."
                className="h-10 w-full rounded-md border border-border/70 bg-card pl-9 pr-3 text-sm text-foreground shadow-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
              />
            </div>
          </div>

          {showFilters ? (
            <div className="mb-6 grid gap-4 rounded-xl border border-border/60 bg-card p-4 md:grid-cols-2">
              <div>
                <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                  Agent
                </label>
                <input
                  placeholder="Filter by agent..."
                  value={agentFilter}
                  onChange={(e) => setAgentFilter(e.target.value)}
                  list="agent-options"
                  className="mt-2 h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground shadow-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                />
                <datalist id="agent-options">
                  {agentOptions.map((a) => (
                    <option key={a} value={a} />
                  ))}
                </datalist>
              </div>
              <div>
                <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                  User
                </label>
                <input
                  placeholder="Filter by user..."
                  value={userFilter}
                  onChange={(e) => setUserFilter(e.target.value)}
                  className="mt-2 h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground shadow-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                />
              </div>
            </div>
          ) : null}

          <div className="grid gap-4 md:grid-cols-3">
            <StatCard
              title="Total threads"
              value={threadsCountValue}
              helper={`Latest activity ${latestActivity}`}
            />
            <StatCard title="Total messages" value={messageCountValue} helper="" />
            <StatCard title="Unique agents" value={uniqueAgentsValue} helper="" />
          </div>

          <div className="mt-6 overflow-hidden rounded-2xl border border-border/70 bg-card shadow-sm">
            <div className="flex items-center justify-between border-b border-border/60 px-6 py-4">
              <h2 className="text-lg font-semibold text-foreground">Recent threads</h2>
              <span className="text-xs font-semibold text-muted-foreground">
                {error ? '—' : `${filtered.length} total`}
              </span>
            </div>
            <div className="divide-y divide-border/60">
              {loading ? (
                <div className="px-6 py-4 text-sm text-muted-foreground">Loading…</div>
              ) : error ? (
                <div className="px-6 py-4 text-sm text-muted-foreground">
                  We couldn't load threads. Please try again shortly.
                </div>
              ) : filtered.length === 0 ? (
                <div className="px-6 py-4 text-sm text-muted-foreground">No threads found.</div>
              ) : (
                filtered.slice(0, visibleCount).map((thread) => (
                  <div
                    key={thread.id}
                    className="group flex items-center justify-between gap-4 px-6 py-4 transition hover:bg-muted/40"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-3">
                        <h3 className="truncate text-base font-medium text-foreground">
                          {thread.title || 'Untitled thread'}
                        </h3>
                        <span className="rounded border border-border/60 bg-muted px-2 py-0.5 text-[10px] font-semibold text-muted-foreground">
                          {thread.message_count ? `${thread.message_count} msgs` : 'No messages'}
                        </span>
                      </div>
                      <div className="mt-2 flex flex-wrap items-center gap-4 text-xs text-muted-foreground">
                        <span>{thread.agent_name || 'Agent'}</span>
                        <span>{formatRelativeTime(thread.updated_at)}</span>
                        <span className="font-mono text-[11px] text-muted-foreground/80">
                          ID: {thread.id}
                        </span>
                      </div>
                    </div>
                    <button
                      type="button"
                      onClick={() => {
                        if (thread.agent_id && thread.id) {
                          navigate(
                            `chat?id=${encodeURIComponent(thread.agent_id)}&threadId=${encodeURIComponent(thread.id)}`
                          );
                        }
                      }}
                      className="flex items-center gap-2 rounded-full p-2 text-muted-foreground text-primary"
                      title="Open thread"
                    >
                      <ArrowRight className="h-4 w-4" />
                    </button>
                  </div>
                ))
              )}
            </div>
          </div>

          {!loading && filtered.length > visibleCount && (
            <div className="mt-6 flex items-center justify-center">
              <button
                type="button"
                onClick={() => setVisibleCount((n) => n + 20)}
                className="rounded-md border border-border/70 bg-card px-4 py-2 text-sm font-medium text-foreground transition hover:bg-muted"
              >
                Load more
              </button>
            </div>
          )}

          <div ref={sentinelRef} />
        </div>
      </div>
    </div>
  );
}

function StatCard({
  title,
  value,
  helper,
}: {
  title: string;
  value: string;
  helper: string;
}) {
  return (
    <div className="rounded-2xl border border-border/70 bg-card p-5 shadow-sm">
      <p className="text-xs font-semibold uppercase tracking-[0.2em] text-muted-foreground">
        {title}
      </p>
      <div className="mt-3 text-3xl font-semibold text-foreground">{value}</div>
      <p className="mt-2 text-xs text-muted-foreground">{helper}</p>
    </div>
  );
}

function formatRelativeTime(value?: string) {
  if (!value) return '—';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '—';
  const deltaMs = Date.now() - date.getTime();
  const minutes = Math.floor(deltaMs / 60000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  return date.toLocaleDateString();
}

function formatNumber(value: number) {
  return new Intl.NumberFormat().format(value);
}
