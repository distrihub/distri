import { useCallback, useMemo, useState } from 'react';
import { useThreads, useAgentsByUsage } from '@distri/react';
import { useDistriHomeNavigate } from '../DistriHomeProvider';
import {
  AlertTriangle,
  ArrowRight,
  ChevronLeft,
  ChevronRight,
  Filter,
  Search,
  X,
  Clock,
  Tag,
} from 'lucide-react';

interface Thread {
  id: string;
  title?: string;
  agent_name?: string;
  agent_id?: string;
  external_id?: string;
  user_name?: string;
  user?: string;
  last_message?: string;
  updated_at?: string;
  message_count?: number;
  tags?: string[];
}

export interface ThreadsViewProps {
  className?: string;
}

type QuickTimeFilter = '5m' | '1h' | '24h' | '7d' | null;

function getTimeFilterDate(filter: QuickTimeFilter): string | undefined {
  if (!filter) return undefined;
  const now = new Date();
  switch (filter) {
    case '5m':
      return new Date(now.getTime() - 5 * 60 * 1000).toISOString();
    case '1h':
      return new Date(now.getTime() - 60 * 60 * 1000).toISOString();
    case '24h':
      return new Date(now.getTime() - 24 * 60 * 60 * 1000).toISOString();
    case '7d':
      return new Date(now.getTime() - 7 * 24 * 60 * 60 * 1000).toISOString();
    default:
      return undefined;
  }
}

export function ThreadsView({ className }: ThreadsViewProps) {
  const navigate = useDistriHomeNavigate();
  const [showFilterDialog, setShowFilterDialog] = useState(false);
  const [searchInput, setSearchInput] = useState('');
  const [quickTimeFilter, setQuickTimeFilter] = useState<QuickTimeFilter>(null);

  // Filter dialog state
  const [dialogAgentId, setDialogAgentId] = useState('');
  const [dialogExternalId, setDialogExternalId] = useState('');
  const [dialogFromDate, setDialogFromDate] = useState('');
  const [dialogToDate, setDialogToDate] = useState('');

  const {
    threads: rawThreads,
    total,
    page,
    pageSize,
    loading,
    error,
    params,
    setParams,
    nextPage,
    prevPage,
    setPageSize,
  } = useThreads();

  const { agents: agentsByUsage } = useAgentsByUsage();

  const threads = rawThreads as unknown as Thread[];

  // Apply search with debounce
  const handleSearchKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'Enter') {
        setParams({ ...params, search: searchInput || undefined, offset: 0 });
      }
    },
    [params, searchInput, setParams]
  );

  const handleSearchClear = useCallback(() => {
    setSearchInput('');
    setParams({ ...params, search: undefined, offset: 0 });
  }, [params, setParams]);

  // Quick time filter handlers
  const handleQuickTimeFilter = useCallback(
    (filter: QuickTimeFilter) => {
      if (quickTimeFilter === filter) {
        // Toggle off
        setQuickTimeFilter(null);
        setParams({ ...params, from_date: undefined, to_date: undefined, offset: 0 });
      } else {
        setQuickTimeFilter(filter);
        setParams({
          ...params,
          from_date: getTimeFilterDate(filter),
          to_date: undefined,
          offset: 0,
        });
      }
    },
    [params, quickTimeFilter, setParams]
  );

  // Page size handler
  const handlePageSizeChange = useCallback(
    (e: React.ChangeEvent<HTMLSelectElement>) => {
      setPageSize(Number(e.target.value));
    },
    [setPageSize]
  );

  // Clickable filter handlers
  const handleAgentClick = useCallback(
    (agentId: string) => {
      setParams({ ...params, agent_id: agentId, offset: 0 });
      setDialogAgentId(agentId);
    },
    [params, setParams]
  );

  const handleExternalIdClick = useCallback(
    (externalId: string) => {
      setParams({ ...params, external_id: externalId, offset: 0 });
      setDialogExternalId(externalId);
    },
    [params, setParams]
  );

  // Filter dialog handlers
  const openFilterDialog = useCallback(() => {
    setDialogAgentId(params.agent_id || '');
    setDialogExternalId(params.external_id || '');
    setDialogFromDate(params.from_date ? params.from_date.split('T')[0] : '');
    setDialogToDate(params.to_date ? params.to_date.split('T')[0] : '');
    setShowFilterDialog(true);
  }, [params]);

  const applyFilters = useCallback(() => {
    setQuickTimeFilter(null); // Clear quick filter when using custom dates
    setParams({
      ...params,
      agent_id: dialogAgentId || undefined,
      external_id: dialogExternalId || undefined,
      from_date: dialogFromDate ? new Date(dialogFromDate).toISOString() : undefined,
      to_date: dialogToDate ? new Date(dialogToDate + 'T23:59:59').toISOString() : undefined,
      offset: 0,
    });
    setShowFilterDialog(false);
  }, [params, dialogAgentId, dialogExternalId, dialogFromDate, dialogToDate, setParams]);

  const clearAllFilters = useCallback(() => {
    setDialogAgentId('');
    setDialogExternalId('');
    setDialogFromDate('');
    setDialogToDate('');
    setQuickTimeFilter(null);
    setSearchInput('');
    setParams({ limit: params.limit, offset: 0 });
    setShowFilterDialog(false);
  }, [params.limit, setParams]);

  // Stats
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

  // Pagination info
  const totalPages = Math.ceil(total / pageSize);
  const hasNextPage = page < totalPages;
  const hasPrevPage = page > 1;

  // Active filters count
  const activeFilterCount = [
    params.agent_id,
    params.external_id,
    params.from_date,
    params.to_date,
    params.search,
  ].filter(Boolean).length;

  const showWarning = Boolean(error);
  const threadsCountValue = loading || error ? '—' : formatNumber(total);
  const messageCountValue = loading || error ? '—' : formatNumber(totalMessages);
  const uniqueAgentsValue = loading || error ? '—' : formatNumber(uniqueAgents);

  return (
    <div className={`flex-1 overflow-y-auto bg-background ${className ?? ''}`}>
      <div className="mx-auto flex w-full max-w-7xl flex-col gap-8 px-4 py-6 sm:px-6 lg:px-8 lg:py-10">
        <section className="flex flex-col gap-6">
          {/* Header */}
          <div className="flex flex-wrap items-center justify-between gap-4">
            <h1 className="text-xl font-semibold text-foreground">Threads</h1>
            <div className="flex flex-wrap items-center gap-3">
              {/* Search */}
              <div className="relative">
                <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
                <input
                  value={searchInput}
                  onChange={(e) => setSearchInput(e.target.value)}
                  onKeyDown={handleSearchKeyDown}
                  placeholder="Search threads... (Enter)"
                  className="h-9 w-64 rounded-md border border-border/70 bg-card pl-9 pr-8 text-sm text-foreground shadow-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                />
                {searchInput && (
                  <button
                    type="button"
                    onClick={handleSearchClear}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                  >
                    <X className="h-4 w-4" />
                  </button>
                )}
              </div>
              {/* Filter button */}
              <div className="flex items-center">
                <button
                  type="button"
                  onClick={openFilterDialog}
                  className={`inline-flex items-center gap-2 border border-border/70 bg-card px-3 py-2 text-sm font-medium text-muted-foreground transition hover:text-primary ${activeFilterCount > 0 ? 'rounded-l-md border-r-0' : 'rounded-md'
                    }`}
                >
                  <Filter className="h-4 w-4" />
                  Filters
                  {activeFilterCount > 0 && (
                    <span className="rounded-full bg-primary px-2 py-0.5 text-xs text-primary-foreground">
                      {activeFilterCount}
                    </span>
                  )}
                </button>
                {activeFilterCount > 0 && (
                  <button
                    type="button"
                    onClick={clearAllFilters}
                    className="inline-flex items-center rounded-r-md border border-border/70 bg-card px-2 py-2 text-muted-foreground transition hover:bg-destructive/10 hover:text-destructive"
                    title="Clear all filters"
                  >
                    <X className="h-4 w-4" />
                  </button>
                )}
              </div>
            </div>
          </div>

          {showWarning && (
            <div className="flex items-start gap-3 rounded-lg border border-amber-300/50 bg-amber-100/60 px-4 py-3 text-xs text-amber-900 dark:border-amber-400/20 dark:bg-amber-400/10 dark:text-amber-100">
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
              <div>
                <p className="font-semibold">We couldn't load threads right now.</p>
                <p className="mt-1 text-amber-800/90 dark:text-amber-100/90">
                  Some stats may be unavailable. Try refreshing soon.
                </p>
              </div>
            </div>
          )}

          {/* Quick time filters */}
          <div className="flex flex-wrap items-center gap-2">
            <Clock className="h-4 w-4 text-muted-foreground" />
            {(['5m', '1h', '24h', '7d'] as const).map((filter) => (
              <button
                key={filter}
                type="button"
                onClick={() => handleQuickTimeFilter(filter)}
                className={`rounded-md px-3 py-1.5 text-xs font-medium transition ${quickTimeFilter === filter
                    ? 'bg-primary text-primary-foreground'
                    : 'border border-border/70 bg-card text-muted-foreground hover:text-foreground'
                  }`}
              >
                {filter === '5m' && 'Last 5 min'}
                {filter === '1h' && 'Last hour'}
                {filter === '24h' && 'Last 24h'}
                {filter === '7d' && 'Last 7 days'}
              </button>
            ))}
            {(params.agent_id || params.external_id || params.search) && (
              <button
                type="button"
                onClick={clearAllFilters}
                className="ml-2 rounded-md px-3 py-1.5 text-xs font-medium text-muted-foreground hover:text-foreground"
              >
                Clear all filters
              </button>
            )}
          </div>

          {/* Stat cards */}
          <div className="grid gap-4 md:grid-cols-3">
            <StatCard
              title="Total threads"
              value={threadsCountValue}
              helper={`Latest activity ${latestActivity}`}
            />
            <StatCard title="Messages on page" value={messageCountValue} helper="" />
            <StatCard title="Agents on page" value={uniqueAgentsValue} helper="" />
          </div>

          {/* Threads list */}
          <div className="rounded-2xl border border-border/70 bg-card shadow-sm">
            {/* List header */}
            <div className="flex items-center justify-between border-b border-border/60 px-6 py-4">
              <h2 className="text-lg font-semibold text-foreground">Threads</h2>
              <div className="flex items-center gap-4">
                <span className="text-xs font-semibold text-muted-foreground">
                  {error ? '—' : `${total} total`}
                </span>
                <select
                  value={pageSize}
                  onChange={handlePageSizeChange}
                  className="h-8 rounded-md border border-border/70 bg-background px-2 text-xs text-foreground"
                >
                  <option value={10}>10 per page</option>
                  <option value={30}>30 per page</option>
                  <option value={100}>100 per page</option>
                </select>
              </div>
            </div>

            {/* Thread list */}
            <div className="divide-y divide-border/60">
              {loading ? (
                <div className="px-6 py-4 text-sm text-muted-foreground">Loading…</div>
              ) : error ? (
                <div className="px-6 py-4 text-sm text-muted-foreground">
                  We couldn't load threads. Please try again shortly.
                </div>
              ) : threads.length === 0 ? (
                <div className="px-6 py-4 text-sm text-muted-foreground">No threads found.</div>
              ) : (
                threads.map((thread) => (
                  <div
                    key={thread.id}
                    onClick={() => {
                      if (thread.agent_id && thread.id) {
                        navigate(
                          `/chat?id=${encodeURIComponent(thread.agent_id)}&threadId=${encodeURIComponent(thread.id)}`
                        );
                      }
                    }}
                    className="group flex cursor-pointer items-center justify-between gap-4 px-6 py-4 transition hover:bg-muted/40"
                  >
                    <div className="min-w-0 flex-1 overflow-hidden">
                      <div className="flex items-baseline gap-3">
                        <h3 className="max-w-[800px] truncate overflow-hidden text-ellipsis text-base font-medium text-foreground">
                          {thread.title || 'Untitled thread'}
                        </h3>
                        <span className="shrink-0 whitespace-nowrap rounded border border-border/60 bg-muted px-2 py-0.5 text-[10px] font-semibold text-muted-foreground">
                          {thread.message_count ? `${thread.message_count} msgs` : 'No messages'}
                        </span>
                      </div>
                      <div className="mt-2 flex flex-wrap items-center gap-4 text-xs text-muted-foreground">
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            thread.agent_id && handleAgentClick(thread.agent_id);
                          }}
                          className="hover:text-primary hover:underline"
                          title={`Filter by agent: ${thread.agent_name || thread.agent_id}`}
                        >
                          {thread.agent_name || 'Agent'}
                        </button>
                        <span>{formatRelativeTime(thread.updated_at)}</span>
                        {thread.external_id && (
                          <button
                            type="button"
                            onClick={(e) => {
                              e.stopPropagation();
                              handleExternalIdClick(thread.external_id!);
                            }}
                            className="font-mono text-[11px] text-muted-foreground/80 hover:text-primary hover:underline"
                            title={`Filter by external ID: ${thread.external_id}`}
                          >
                            ext:{thread.external_id}
                          </button>
                        )}
                        <span className="font-mono text-[11px] text-muted-foreground/80">
                          ID: {thread.id.slice(0, 8)}...
                        </span>
                      </div>
                      {thread.tags && thread.tags.length > 0 && (
                        <div className="mt-2 flex flex-wrap gap-1">
                          {thread.tags.map((tag) => (
                            <span
                              key={tag}
                              className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-[10px] font-medium text-muted-foreground"
                            >
                              <Tag className="h-2.5 w-2.5" />
                              {tag}
                            </span>
                          ))}
                        </div>
                      )}
                    </div>
                    <button
                      type="button"
                      onClick={(e) => {
                        e.stopPropagation();
                        if (thread.agent_id && thread.id) {
                          navigate(
                            `/chat?id=${encodeURIComponent(thread.agent_id)}&threadId=${encodeURIComponent(thread.id)}`
                          );
                        }
                      }}
                      className="flex items-center gap-2 rounded-full p-2 text-primary"
                      title="Open thread"
                    >
                      <ArrowRight className="h-4 w-4" />
                    </button>
                  </div>
                ))
              )}
            </div>

            {/* Pagination controls */}
            {!loading && !error && total > 0 && (
              <div className="flex items-center justify-between border-t border-border/60 px-6 py-3">
                <span className="text-xs text-muted-foreground">
                  Page {page} of {totalPages}
                </span>
                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={prevPage}
                    disabled={!hasPrevPage}
                    className="inline-flex items-center gap-1 rounded-md border border-border/70 bg-card px-3 py-1.5 text-sm font-medium text-foreground transition hover:bg-muted disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    <ChevronLeft className="h-4 w-4" />
                    Prev
                  </button>
                  <button
                    type="button"
                    onClick={nextPage}
                    disabled={!hasNextPage}
                    className="inline-flex items-center gap-1 rounded-md border border-border/70 bg-card px-3 py-1.5 text-sm font-medium text-foreground transition hover:bg-muted disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    Next
                    <ChevronRight className="h-4 w-4" />
                  </button>
                </div>
              </div>
            )}
          </div>
        </section>
      </div>

      {/* Filter Dialog Modal */}
      {showFilterDialog && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="w-full max-w-md rounded-xl border border-border bg-card p-6 shadow-xl">
            <div className="mb-6 flex items-center justify-between">
              <h2 className="text-lg font-semibold text-foreground">Filter Threads</h2>
              <button
                type="button"
                onClick={() => setShowFilterDialog(false)}
                className="text-muted-foreground hover:text-foreground"
              >
                <X className="h-5 w-5" />
              </button>
            </div>

            <div className="space-y-4">
              {/* Agent dropdown */}
              <div>
                <label className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                  Agent
                </label>
                <select
                  value={dialogAgentId}
                  onChange={(e) => setDialogAgentId(e.target.value)}
                  className="h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground"
                >
                  <option value="">All agents</option>
                  {agentsByUsage.map((agent) => (
                    <option key={agent.agent_id} value={agent.agent_id}>
                      {agent.agent_name} ({agent.thread_count} threads)
                    </option>
                  ))}
                </select>
              </div>

              {/* External ID */}
              <div>
                <label className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                  External ID
                </label>
                <input
                  type="text"
                  value={dialogExternalId}
                  onChange={(e) => setDialogExternalId(e.target.value)}
                  placeholder="Filter by external ID..."
                  className="h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground"
                />
              </div>

              {/* Date range */}
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                    From Date
                  </label>
                  <input
                    type="date"
                    value={dialogFromDate}
                    onChange={(e) => setDialogFromDate(e.target.value)}
                    className="h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground"
                  />
                </div>
                <div>
                  <label className="mb-2 block text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                    To Date
                  </label>
                  <input
                    type="date"
                    value={dialogToDate}
                    onChange={(e) => setDialogToDate(e.target.value)}
                    className="h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground"
                  />
                </div>
              </div>
            </div>

            <div className="mt-6 flex items-center justify-between">
              <button
                type="button"
                onClick={clearAllFilters}
                className="text-sm text-muted-foreground hover:text-foreground"
              >
                Clear all
              </button>
              <div className="flex gap-2">
                <button
                  type="button"
                  onClick={() => setShowFilterDialog(false)}
                  className="rounded-md border border-border/70 bg-card px-4 py-2 text-sm font-medium text-foreground transition hover:bg-muted"
                >
                  Cancel
                </button>
                <button
                  type="button"
                  onClick={applyFilters}
                  className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition hover:bg-primary/90"
                >
                  Apply Filters
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
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
      {helper && <p className="mt-2 text-xs text-muted-foreground">{helper}</p>}
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
