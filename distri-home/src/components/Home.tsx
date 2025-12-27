import { useMemo, useState } from 'react';
import { useAgentDefinitions } from '@distri/react';
import { useDistriHomeNavigate } from '../DistriHomeProvider';
import { useHomeStats } from '../hooks/useHomeStats';
import { HomeStatsThread } from '../DistriHomeClient';
import {
  AlertTriangle,
  ArrowUpRight,
  Bot,
  Gauge,
  Loader2,
  MessageSquare,
  Plus,
  RefreshCw,
  Users,
  X,
} from 'lucide-react';

export interface HomeProps {
  /**
   * Callback when "New agent" button is clicked
   * If not provided, shows the AgentPushHelp dialog
   */
  onNewAgent?: () => void;
  /**
   * Custom render for the "new agent" action
   */
  renderNewAgentHelp?: (props: { open: boolean; onOpenChange: (open: boolean) => void }) => React.ReactNode;
  /**
   * Optional custom class name
   */
  className?: string;
}

export function Home({ onNewAgent, renderNewAgentHelp, className }: HomeProps) {
  const navigate = useDistriHomeNavigate();
  const { stats, loading: statsLoading, error: statsError, refetch } = useHomeStats();
  const { agents } = useAgentDefinitions();
  const [refreshing, setRefreshing] = useState(false);
  const [showPushHelp, setShowPushHelp] = useState(false);

  // Suppress unused variable warning
  void agents;

  const latestThreads = (stats?.latest_threads ?? []) as HomeStatsThread[];
  const mostActiveAgent = stats?.most_active_agent ?? null;

  const latestActivityLabel = useMemo(() => {
    if (statsError) return 'Unavailable';
    if (!latestThreads[0]?.updated_at) return '—';
    return formatRelativeTime(latestThreads[0].updated_at);
  }, [latestThreads, statsError]);

  const showWarning = Boolean(statsError);
  const ownedAgents = stats?.total_owned_agents;
  const accessibleAgents = stats?.total_accessible_agents ?? stats?.total_agents;
  const agentCountValue =
    statsLoading || accessibleAgents == null
      ? '—'
      : ownedAgents != null
        ? `${formatNumber(ownedAgents)} / ${formatNumber(accessibleAgents)}`
        : formatNumber(accessibleAgents);
  const threadsCountValue =
    statsLoading || stats?.total_threads == null ? '—' : formatNumber(stats.total_threads);
  const messageCountValue =
    statsLoading || stats?.total_messages == null ? '—' : formatNumber(stats.total_messages);
  const mostActiveLabel = mostActiveAgent?.name || '—';
  const avgTimeLabel =
    statsLoading || stats?.avg_time_per_run_ms == null
      ? '—'
      : `${Math.round((stats.avg_time_per_run_ms / 1000) * 10) / 10}s`;

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      await refetch();
    } finally {
      setRefreshing(false);
    }
  };

  const handleNewAgent = () => {
    if (onNewAgent) {
      onNewAgent();
    } else {
      setShowPushHelp(true);
    }
  };

  return (
    <div className={`flex-1 overflow-y-auto ${className ?? ''}`}>
      <div className="mx-auto w-full max-w-6xl px-6 py-8 lg:px-10">
        <header className="mb-8 flex flex-wrap items-center justify-between gap-4">
          <div />
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={handleRefresh}
              className="inline-flex items-center gap-2 rounded-lg border border-border/70 bg-card px-4 py-2 text-sm font-medium text-foreground transition hover:border-primary/40 hover:text-primary"
            >
              <RefreshCw className={refreshing ? 'h-4 w-4 animate-spin' : 'h-4 w-4'} />
              {refreshing ? 'Refreshing' : 'Refresh'}
            </button>
            <button
              type="button"
              onClick={handleNewAgent}
              className="inline-flex items-center gap-2 rounded-lg bg-primary px-4 py-2 text-sm font-semibold text-primary-foreground shadow-sm shadow-primary/20 transition hover:bg-primary/90"
            >
              <Plus className="h-4 w-4" />
              New agent
            </button>
          </div>
        </header>

        {showWarning ? (
          <div className="mb-6 flex items-start gap-3 rounded-lg border border-amber-300/50 bg-amber-100/60 px-4 py-3 text-xs text-amber-900 dark:border-amber-400/20 dark:bg-amber-400/10 dark:text-amber-100">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <div>
              <p className="font-semibold">We couldn't connect to the server.</p>
              <p className="mt-1 text-amber-800/90 dark:text-amber-100/90">
                Some data may be unavailable right now. Try refreshing in a moment.
              </p>
            </div>
          </div>
        ) : null}

        <div className="grid gap-6 lg:grid-cols-3">
          <div className="relative overflow-hidden rounded-2xl border border-border/70 bg-card p-6 shadow-sm lg:col-span-2">
            <div className="absolute right-4 top-4 text-primary/10">
              <Gauge className="h-20 w-20" />
            </div>
            <div className="relative z-10">
              <div className="flex items-center gap-2 text-lg font-semibold text-foreground">
                <Gauge className="h-4 w-4 text-primary" />
                Overview
              </div>
              <div className="mt-6 grid gap-6 md:grid-cols-3">
                <OverviewStat
                  label="Total messages"
                  value={messageCountValue}
                  helper={statsLoading || statsError ? 'Unavailable' : 'Across all threads'}
                />
                <OverviewStat
                  label="Total threads"
                  value={threadsCountValue}
                  helper={`Latest ${latestActivityLabel}`}
                  className="md:border-l md:border-border/60 md:pl-6"
                />
                <OverviewStat
                  label="Avg time per run"
                  value={avgTimeLabel}
                  helper={statsLoading || statsError ? 'Unavailable' : 'Across all runs'}
                  className="md:border-l md:border-border/60 md:pl-6"
                />
              </div>
            </div>
          </div>

          <div className="flex flex-col justify-between rounded-2xl border border-border/70 bg-card p-6 shadow-sm">
            <div>
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2 text-lg font-semibold text-foreground">
                  <Users className="h-4 w-4 text-primary" />
                  Agents
                </div>
                <button
                  type="button"
                  onClick={() => navigate('agents')}
                  className="rounded-md border border-primary/20 bg-primary/5 px-2 py-1 text-xs font-semibold text-primary transition hover:bg-primary/10"
                >
                  View all
                </button>
              </div>
              <div className="mt-5 text-4xl font-semibold text-foreground">
                {agentCountValue}
              </div>
              <p className="mt-2 text-sm text-muted-foreground">
                {statsLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : 'Owned / All agents'}
              </p>
            </div>
            <div className="mt-6 border-t border-border/60 pt-4 text-sm text-muted-foreground">
              <div className="flex items-center justify-between">
                <span>Most active</span>
                {statsError ? (
                  <span className="font-medium text-foreground">Unavailable</span>
                ) : mostActiveAgent?.id ? (
                  <button
                    type="button"
                    onClick={() => navigate(`agents/${encodeURIComponent(mostActiveAgent.id)}`)}
                    className="font-medium text-primary transition hover:text-primary/80"
                  >
                    {mostActiveLabel}
                  </button>
                ) : (
                  <span className="font-medium text-foreground">{mostActiveLabel}</span>
                )}
              </div>
            </div>
          </div>
        </div>

        <div className="mt-8 rounded-2xl border border-border/70 bg-card shadow-sm">
          <div className="flex items-center justify-between border-b border-border/60 px-6 py-4">
            <div className="flex items-center gap-2 text-lg font-semibold text-foreground">
              <MessageSquare className="h-4 w-4 text-primary" />
              Latest threads
            </div>
            <button
              type="button"
              onClick={() => navigate('threads')}
              className="rounded-lg border border-primary/20 bg-primary/5 px-3 py-1.5 text-xs font-semibold text-primary transition hover:bg-primary/10"
            >
              View all
            </button>
          </div>
          <div className="divide-y divide-border/60">
            {statsLoading ? (
              <div className="px-6 py-4 text-sm text-muted-foreground">Loading…</div>
            ) : statsError ? (
              <div className="px-6 py-4 text-sm text-muted-foreground">
                We couldn't load threads right now.
              </div>
            ) : latestThreads.length === 0 ? (
              <div className="px-6 py-4 text-sm text-muted-foreground">No conversations yet.</div>
            ) : (
              latestThreads.map((thread: HomeStatsThread, index: number) => {
                const avatarStyle = threadAvatarStyles[index % threadAvatarStyles.length];
                return (
                  <div
                    key={thread.id}
                    className="group flex items-center justify-between gap-4 px-6 py-4 transition hover:bg-muted/40"
                  >
                    <div className="flex min-w-0 items-start gap-3">
                      <div
                        className={`flex h-9 w-9 items-center justify-center rounded-full ${avatarStyle.bg} ${avatarStyle.text}`}
                      >
                        <Bot className="h-4 w-4" />
                      </div>
                      <div className="min-w-0">
                        <p className="truncate text-sm font-semibold text-foreground group-hover:text-primary">
                          {thread.title || 'Untitled thread'}
                        </p>
                        <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                          <span className="rounded border border-border/60 bg-muted/60 px-1.5 py-0.5">
                            {thread.agent_name || 'Unknown agent'}
                          </span>
                          <span>• {formatRelativeTime(thread.updated_at)}</span>
                        </div>
                      </div>
                    </div>
                    <button
                      type="button"
                      onClick={() => {
                        if (thread.agent_id && thread.id) {
                          navigate(
                            `chat?id=${encodeURIComponent(thread.agent_id)}&threadId=${encodeURIComponent(thread.id)}`
                          );
                        } else {
                          navigate('threads');
                        }
                      }}
                      className="flex items-center gap-1 rounded-md border border-border/70 bg-background px-3 py-1.5 text-xs font-semibold text-foreground opacity-0 transition group-hover:opacity-100 hover:border-primary/50 hover:text-primary"
                    >
                      Open
                      <ArrowUpRight className="h-3 w-3" />
                    </button>
                  </div>
                );
              })
            )}
          </div>
        </div>
      </div>

      {/* Default push help dialog - can be replaced via renderNewAgentHelp prop */}
      {renderNewAgentHelp ? (
        renderNewAgentHelp({ open: showPushHelp, onOpenChange: setShowPushHelp })
      ) : (
        <DefaultAgentPushHelp open={showPushHelp} onOpenChange={setShowPushHelp} />
      )}
    </div>
  );
}

// Default built-in help dialog (can be overridden via renderNewAgentHelp prop)
function DefaultAgentPushHelp({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div
        className="absolute inset-0 bg-black/50"
        onClick={() => onOpenChange(false)}
        aria-hidden="true"
      />
      <div className="relative z-10 w-full max-w-md rounded-2xl border border-border bg-card p-6 shadow-lg">
        <div className="flex items-center justify-between">
          <h3 className="text-lg font-semibold text-foreground">Push an agent with distri</h3>
          <button
            type="button"
            onClick={() => onOpenChange(false)}
            className="rounded-lg p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <X className="h-5 w-5" />
          </button>
        </div>
        <p className="mt-2 text-sm text-muted-foreground">
          Use the CLI to register an agent markdown file.
        </p>
        <div className="mt-4 space-y-3 text-sm">
          <p>Example:</p>
          <pre className="rounded-md bg-muted/50 p-3 text-xs">
            {`distri push "agents/my-agent.md"`}
          </pre>
          <p className="text-xs text-muted-foreground">
            See the docs for full reference.
          </p>
          <a
            href="https://distri.dev/docs/"
            target="_blank"
            rel="noreferrer"
            className="inline-flex w-full items-center justify-center rounded-md border border-border/70 bg-card px-4 py-2 text-sm font-medium text-foreground transition hover:bg-muted"
          >
            Open documentation
          </a>
        </div>
      </div>
    </div>
  );
}

const threadAvatarStyles = [
  { bg: 'bg-sky-100 dark:bg-sky-500/20', text: 'text-sky-600 dark:text-sky-300' },
  { bg: 'bg-purple-100 dark:bg-purple-500/20', text: 'text-purple-600 dark:text-purple-300' },
  { bg: 'bg-emerald-100 dark:bg-emerald-500/20', text: 'text-emerald-600 dark:text-emerald-300' },
];

function OverviewStat({
  label,
  value,
  helper,
  className,
}: {
  label: string;
  value: string | number;
  helper?: string;
  className?: string;
}) {
  return (
    <div className={className}>
      <div className="text-xs font-semibold uppercase tracking-[0.2em] text-muted-foreground">{label}</div>
      <div className="mt-2 text-3xl font-semibold text-foreground">{value}</div>
      {helper ? <div className="mt-2 text-xs text-muted-foreground">{helper}</div> : null}
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
