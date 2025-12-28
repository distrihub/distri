import { useState, useEffect, KeyboardEvent } from 'react';
import { useDistriHome } from '../DistriHomeProvider';
import { SessionSummary } from '../DistriHomeClient';
import { Loader2, ArrowLeft, ArrowRight, Search, Clock } from 'lucide-react';

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

  return (
    <div className={`flex flex-col space-y-6 px-6 py-8 lg:px-10 w-full ${className ?? ''}`}>
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Sessions</h1>
        <p className="text-muted-foreground">Manage and inspect active sessions.</p>
      </div>

      <div className="flex items-center gap-4">
        <div className="relative flex-1">
          <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <input
            type="text"
            placeholder="Filter by Thread ID..."
            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 pl-9 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            value={filterInput}
            onChange={(e) => setFilterInput(e.target.value)}
            onKeyDown={handleKeyDown}
          />
        </div>
        <button
          onClick={handleSearch}
          className="inline-flex h-9 items-center justify-center rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground shadow transition-colors hover:bg-primary/90 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          Search
        </button>
      </div>

      {error && (
        <div className="rounded-md bg-destructive/15 p-3 text-sm text-destructive">
          Error: {error}
        </div>
      )}

      <div className="rounded-md border border-border">
        <div className="relative w-full overflow-auto">
          <table className="w-full caption-bottom text-sm">
            <thead className="[&_tr]:border-b">
              <tr className="border-b transition-colors hover:bg-muted/50 data-[state=selected]:bg-muted">
                <th className="h-10 px-4 text-left align-middle font-medium text-muted-foreground">
                  Thread ID
                </th>
                <th className="h-10 px-4 text-left align-middle font-medium text-muted-foreground">
                  Keys
                </th>
                <th className="h-10 px-4 text-left align-middle font-medium text-muted-foreground">
                  Updated
                </th>
              </tr>
            </thead>
            <tbody className="[&_tr:last-child]:border-0">
              {loading && sessions.length === 0 ? (
                <tr>
                  <td colSpan={3} className="h-24 text-center">
                    <Loader2 className="mx-auto h-6 w-6 animate-spin text-muted-foreground" />
                  </td>
                </tr>
              ) : sessions.length === 0 ? (
                <tr>
                  <td colSpan={3} className="h-24 text-center text-muted-foreground">
                    No sessions found.
                  </td>
                </tr>
              ) : (
                sessions.map((session) => (
                  <tr
                    key={session.session_id}
                    className="border-b transition-colors hover:bg-muted/50 data-[state=selected]:bg-muted"
                  >
                    <td className="p-4 align-middle font-mono text-xs">
                      {session.session_id}
                    </td>
                    <td className="p-4 align-middle">
                      <div className="flex flex-wrap gap-1">
                        {session.keys.length > 0 ? (
                          session.keys.slice(0, 5).map(k => (
                            <span key={k} className="inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-semibold transition-colors border-transparent bg-secondary text-secondary-foreground hover:bg-secondary/80">
                              {k}
                            </span>
                          ))
                        ) : (
                          <span className="text-muted-foreground italic">No keys</span>
                        )}
                        {session.keys.length > 5 && (
                          <span className="text-xs text-muted-foreground">+{session.keys.length - 5} more</span>
                        )}
                      </div>
                    </td>
                    <td className="p-4 align-middle text-muted-foreground">
                      {session.updated_at ? (
                        <span className="flex items-center gap-1" title={session.updated_at}>
                          <Clock className="h-3 w-3" />
                          {timeAgo(session.updated_at)}
                        </span>
                      ) : (
                        '-'
                      )}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </div>

      <div className="flex items-center justify-between">
        <div className="text-sm text-muted-foreground">
          Showing {sessions.length} results
        </div>
        <div className="flex items-center space-x-2">
          <button
            className="inline-flex h-8 items-center justify-center rounded-md border border-input bg-background px-3 text-xs font-medium shadow-sm transition-colors hover:bg-accent hover:text-accent-foreground disabled:pointer-events-none disabled:opacity-50"
            disabled={offset === 0 || loading}
            onClick={() => setOffset((prev) => Math.max(0, prev - limit))}
          >
            <ArrowLeft className="mr-2 h-4 w-4" />
            Previous
          </button>
          <button
            className="inline-flex h-8 items-center justify-center rounded-md border border-input bg-background px-3 text-xs font-medium shadow-sm transition-colors hover:bg-accent hover:text-accent-foreground disabled:pointer-events-none disabled:opacity-50"
            disabled={sessions.length < limit || loading}
            onClick={() => setOffset((prev) => prev + limit)}
          >
            Next
            <ArrowRight className="ml-2 h-4 w-4" />
          </button>
        </div>
      </div>
    </div>
  );
}
