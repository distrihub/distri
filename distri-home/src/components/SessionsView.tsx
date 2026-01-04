import { useState, useEffect, KeyboardEvent } from 'react';
import { useDistriHome } from '../DistriHomeProvider';
import { SessionSummary } from '../DistriHomeClient';
import { Loader2, ArrowLeft, ArrowRight, Search, Clock, X, Database } from 'lucide-react';

export interface SessionsViewProps {
  className?: string;
}

function timeAgo(dateStr: string) {
  const date = new Date(dateStr);
  const now = new Date();
  const seconds = Math.floor((now.getTime() - date.getTime()) / 1000);

  const rtf = new Intl.RelativeTimeFormat('en', { numeric: 'auto' });

  if (seconds < 60) return rtf.format(-seconds, 'second');
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return rtf.format(-minutes, 'minute');
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return rtf.format(-hours, 'hour');
  const days = Math.floor(hours / 24);
  return rtf.format(-days, 'day');
}

export function SessionsView({ className }: SessionsViewProps) {
  const { homeClient: client } = useDistriHome();
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Pagination & Filter
  const [limit] = useState(20);
  const [offset, setOffset] = useState(0);
  const [filterInput, setFilterInput] = useState('');
  const [activeFilter, setActiveFilter] = useState('');

  // Details View
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [sessionDetails, setSessionDetails] = useState<Record<string, any> | null>(null);
  const [loadingDetails, setLoadingDetails] = useState(false);

  useEffect(() => {
    if (!selectedSessionId || !client) {
      setSessionDetails(null);
      return;
    }
    const loadDetails = async () => {
      setLoadingDetails(true);
      try {
        const data = await client.getSessionValues(selectedSessionId);
        setSessionDetails(data);
      } catch (e) {
        console.error("Failed to load session details", e);
      } finally {
        setLoadingDetails(false);
      }
    };
    loadDetails();
  }, [selectedSessionId, client]);

  const fetchSessions = async () => {
    if (!client) return;
    setLoading(true);
    setError(null);
    try {
      const data = await client.listSessions({
        threadId: activeFilter || undefined,
        limit,
        offset,
      });
      setSessions(data);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Unknown error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchSessions();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeFilter, offset, client]);

  const handleSearch = () => {
    setOffset(0);
    setActiveFilter(filterInput);
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      handleSearch();
    }
  };

  if (!client) {
    return <div className="p-6 text-destructive">Error: Client not initialized</div>;
  }

  const handleClearFilter = () => {
    setFilterInput('');
    setActiveFilter('');
    setOffset(0);
  };

  return (
    <div className={`flex-1 overflow-y-auto bg-background ${className ?? ''}`}>
      <div className="mx-auto flex w-full max-w-7xl flex-col gap-8 px-4 py-6 sm:px-6 lg:px-8 lg:py-10">
        <section className="flex flex-col gap-6">
          {/* Header */}
          <div className="flex flex-wrap items-center justify-between gap-4">
            <h1 className="text-xl font-semibold text-foreground">Sessions</h1>
            <div className="flex flex-wrap items-center gap-3">
              <div className="relative">
                <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
                <input
                  type="text"
                  placeholder="Filter by Thread ID... (Enter)"
                  className="h-9 w-64 rounded-md border border-border/70 bg-card pl-9 pr-8 text-sm text-foreground shadow-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                  value={filterInput}
                  onChange={(e) => setFilterInput(e.target.value)}
                  onKeyDown={handleKeyDown}
                />
                {filterInput && (
                  <button
                    type="button"
                    onClick={handleClearFilter}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                  >
                    <X className="h-4 w-4" />
                  </button>
                )}
              </div>
            </div>
          </div>

          {error && (
            <div className="rounded-lg border border-amber-300/50 bg-amber-100/60 px-4 py-3 text-xs text-amber-900 dark:border-amber-400/20 dark:bg-amber-400/10 dark:text-amber-100">
              Error: {error}
            </div>
          )}

          {/* Sessions table */}
          <div className="rounded-2xl border border-border/70 bg-card shadow-sm">
            {/* Table header */}
            <div className="flex items-center justify-between border-b border-border/60 px-6 py-4">
              <h2 className="text-lg font-semibold text-foreground">Sessions</h2>
              <span className="text-xs font-semibold text-muted-foreground">
                {sessions.length} results
              </span>
            </div>

            {/* Table */}
            <div className="divide-y divide-border/60">
              {loading && sessions.length === 0 ? (
                <div className="flex items-center justify-center py-12">
                  <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
                </div>
              ) : sessions.length === 0 ? (
                <div className="px-6 py-4 text-sm text-muted-foreground">No sessions found.</div>
              ) : (
                sessions.map((session) => (
                  <div
                    key={session.session_id}
                    className="group flex cursor-pointer items-center justify-between gap-4 px-6 py-4 transition hover:bg-muted/40"
                    onClick={() => setSelectedSessionId(session.session_id)}
                  >
                    <div className="min-w-0 flex-1">
                      <div className="flex items-baseline gap-3">
                        <span className="font-mono text-sm text-foreground">
                          {session.session_id}
                        </span>
                      </div>
                      <div className="mt-2 flex flex-wrap items-center gap-2">
                        {session.keys.length > 0 ? (
                          <>
                            {session.keys.slice(0, 5).map(k => (
                              <span key={k} className="rounded border border-border/60 bg-muted px-2 py-0.5 text-[10px] font-semibold text-muted-foreground">
                                {k}
                              </span>
                            ))}
                            {session.keys.length > 5 && (
                              <span className="text-xs text-muted-foreground">+{session.keys.length - 5} more</span>
                            )}
                          </>
                        ) : (
                          <span className="text-xs text-muted-foreground italic">No keys</span>
                        )}
                        {session.updated_at && (
                          <span className="flex items-center gap-1 text-xs text-muted-foreground" title={session.updated_at}>
                            <Clock className="h-3 w-3" />
                            {timeAgo(session.updated_at)}
                          </span>
                        )}
                      </div>
                    </div>
                    <ArrowRight className="h-4 w-4 text-primary" />
                  </div>
                ))
              )}
            </div>

            {/* Pagination controls */}
            <div className="flex items-center justify-between border-t border-border/60 px-6 py-3">
              <span className="text-xs text-muted-foreground">
                Page {Math.floor(offset / limit) + 1}
              </span>
              <div className="flex items-center gap-2">
                <button
                  className="inline-flex items-center gap-1 rounded-md border border-border/70 bg-card px-3 py-1.5 text-sm font-medium text-foreground transition hover:bg-muted disabled:cursor-not-allowed disabled:opacity-50"
                  disabled={offset === 0 || loading}
                  onClick={() => setOffset((prev) => Math.max(0, prev - limit))}
                >
                  <ArrowLeft className="h-4 w-4" />
                  Prev
                </button>
                <button
                  className="inline-flex items-center gap-1 rounded-md border border-border/70 bg-card px-3 py-1.5 text-sm font-medium text-foreground transition hover:bg-muted disabled:cursor-not-allowed disabled:opacity-50"
                  disabled={sessions.length < limit || loading}
                  onClick={() => setOffset((prev) => prev + limit)}
                >
                  Next
                  <ArrowRight className="h-4 w-4" />
                </button>
              </div>
            </div>
          </div>
        </section>
      </div>

      {selectedSessionId && (
        <div className="fixed inset-0 z-50 flex justify-end bg-background/80 backdrop-blur-sm" onClick={() => setSelectedSessionId(null)}>
          <div
            className="w-full max-w-2xl border-l bg-background p-6 shadow-2xl overflow-y-auto animate-in slide-in-from-right duration-200"
            onClick={e => e.stopPropagation()}
          >
            <div className="flex items-center justify-between mb-6">
              <div>
                <h2 className="text-lg font-semibold flex items-center gap-2">
                  <Database className="h-5 w-5" />
                  Session Details
                </h2>
                <p className="text-sm text-muted-foreground font-mono mt-1">{selectedSessionId}</p>
              </div>
              <button
                onClick={() => setSelectedSessionId(null)}
                className="rounded-full p-2 hover:bg-muted transition-colors"
              >
                <X className="h-5 w-5" />
              </button>
            </div>

            {loadingDetails ? (
              <div className="flex items-center justify-center h-40">
                <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
              </div>
            ) : sessionDetails ? (
              <div className="space-y-6">
                {Object.entries(sessionDetails).map(([key, value]) => (
                  <div key={key} className="space-y-2">
                    <h3 className="text-sm font-medium text-foreground bg-muted/40 px-2 py-1 rounded inline-block">
                      {key}
                    </h3>
                    <div className="rounded-md border bg-muted/20 p-4 overflow-x-auto">
                      <pre className="text-xs font-mono whitespace-pre-wrap break-all">
                        {typeof value === 'string' ? value : JSON.stringify(value, null, 2)}
                      </pre>
                    </div>
                  </div>
                ))}
                {Object.keys(sessionDetails).length === 0 && (
                  <div className="text-muted-foreground text-center py-10">No values stored in this session.</div>
                )}
              </div>
            ) : (
              <div className="text-destructive">Failed to load details.</div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
