import { useMemo, useState } from 'react';
import { useAgentDefinitions } from '@distri/react';
import { useDistriHomeNavigate, useDistriHomeConfig } from '../DistriHomeProvider';
import { useHomeStats } from '../hooks/useHomeStats';
import { HomeStatsThread, RecentlyUsedAgent } from '../DistriHomeClient';
import {
  AlertTriangle,
  ArrowUpRight,
  Bot,
  Clock,
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
  const config = useDistriHomeConfig();
  const { stats, loading: statsLoading, error: statsError, refetch } = useHomeStats();
  const { agents } = useAgentDefinitions();
  const [refreshing, setRefreshing] = useState(false);
  const [showPushHelp, setShowPushHelp] = useState(false);

  // Suppress unused variable warning
  void agents;

  const latestThreads = (stats?.latest_threads ?? []) as HomeStatsThread[];
  const mostActiveAgent = stats?.most_active_agent ?? null;
  // Show first 5 of the recently used agents
  const recentlyUsedAgents = (stats?.recently_used_agents ?? []).slice(0, 5) as RecentlyUsedAgent[];

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
    statsLoading || stats?.avg_run_time_ms == null
      ? '—'
      : `${Math.round((stats.avg_run_time_ms / 1000) * 10) / 10}s`;

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
    <div className={`flex-1 overflow-y-auto bg-background ${className ?? ''}`}>
      <div className="mx-auto w-full max-w-7xl px-4 py-6 sm:px-6 lg:px-8 lg:py-10">
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

        {/* Stats Overview - Single Row */}
        <div className="relative overflow-hidden rounded-2xl border border-border/70 bg-card p-6 shadow-sm">
          <div className="absolute right-4 top-4 text-primary/10">
            <Gauge className="h-20 w-20" />
          </div>
          <div className="relative z-10">
            <div className="flex items-center gap-2 text-lg font-semibold text-foreground">
              <Gauge className="h-4 w-4 text-primary" />
              Overview
            </div>
            <div className="mt-6 grid gap-6 grid-cols-2 md:grid-cols-5">
              <OverviewStat
                label="Messages"
                value={messageCountValue}
                helper={statsLoading || statsError ? 'Unavailable' : 'Across all threads'}
              />
              <OverviewStat
                label="Threads"
                value={threadsCountValue}
                helper={`Latest ${latestActivityLabel}`}
                className="md:border-l md:border-border/60 md:pl-6"
              />
              <OverviewStat
                label="Avg run time"
                value={avgTimeLabel}
                helper={statsLoading || statsError ? 'Unavailable' : 'Across all runs'}
                className="md:border-l md:border-border/60 md:pl-6"
              />
              <OverviewStat
                label="Agents"
                value={agentCountValue}
                helper={statsLoading || statsError ? 'Unavailable' : 'Owned / All'}
                className="md:border-l md:border-border/60 md:pl-6"
              />
              <div className="md:border-l md:border-border/60 md:pl-6">
                <div className="text-xs font-semibold uppercase tracking-[0.2em] text-muted-foreground">Most active</div>
                <div className="mt-2">
                  {statsError ? (
                    <span className="text-xl font-semibold text-foreground">—</span>
                  ) : mostActiveAgent?.id ? (
                    <button
                      type="button"
                      onClick={() => {
                        const getPath = config.navigationPaths?.agentDetails;
                        const path = getPath
                          ? getPath(mostActiveAgent.id)
                          : `agents/${encodeURIComponent(mostActiveAgent.id)}`;
                        navigate(path);
                      }}
                      className="text-xl font-semibold text-primary transition hover:text-primary/80"
                    >
                      {mostActiveLabel}
                    </button>
                  ) : (
                    <span className="text-xl font-semibold text-foreground">{mostActiveLabel}</span>
                  )}
                </div>
                <div className="mt-2 text-xs text-muted-foreground">
                  {statsLoading || statsError ? 'Unavailable' : `${mostActiveAgent?.count ?? 0} threads`}
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* Latest Threads and Recently Used Agents - aligned with top row */}
        <div className="mt-6 grid gap-6 lg:grid-cols-3">
          {/* Latest Threads - spans 2 columns to align with Overview */}
          <div className="rounded-2xl border border-border/70 bg-card shadow-sm lg:col-span-2">
            <div className="flex items-center justify-between border-b border-border/60 px-6 py-4">
              <div className="flex items-center gap-2 text-lg font-semibold text-foreground">
                <MessageSquare className="h-4 w-4 text-primary" />
                Latest threads
              </div>
              <button
                type="button"
                onClick={() => navigate('threads')}
                className="rounded-md border border-primary/20 bg-primary/5 px-2 py-1 text-xs font-semibold text-primary transition hover:bg-primary/10"
              >
                View all
              </button>
            </div>
            <div className="divide-y divide-border/60">
              {statsLoading ? (
                <div className="flex items-center justify-center py-12 text-sm text-muted-foreground">
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Loading…
                </div>
              ) : statsError ? (
                <div className="py-12 text-center text-sm text-muted-foreground">
                  We couldn't load threads right now.
                </div>
              ) : latestThreads.length === 0 ? (
                <div className="py-12 text-center text-sm text-muted-foreground">No conversations yet.</div>
              ) : (
                latestThreads.slice(0, 4).map((thread: HomeStatsThread, index: number) => {
                  const avatarStyle = threadAvatarStyles[index % threadAvatarStyles.length];
                  return (
                    <button
                      key={thread.id}
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
                      className="group flex w-full items-center gap-4 px-6 py-3.5 text-left transition hover:bg-muted/40"
                    >
                      <div
                        className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-full ${avatarStyle.bg} ${avatarStyle.text}`}
                      >
                        <MessageSquare className="h-3.5 w-3.5" />
                      </div>
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-sm font-medium text-foreground group-hover:text-primary">
                          {thread.title || 'Untitled thread'}
                        </p>
                        <p className="mt-0.5 text-xs text-muted-foreground">
                          {thread.agent_name || 'Unknown agent'} · {formatRelativeTime(thread.updated_at)}
                        </p>
                      </div>
                      <ArrowUpRight className="h-4 w-4 shrink-0 text-muted-foreground opacity-0 transition group-hover:opacity-100" />
                    </button>
                  );
                })
              )}
            </div>
          </div>

          {/* Recently Active Agents */}
          <div className="flex flex-col rounded-2xl border border-border/70 bg-card shadow-sm">
            <div className="flex items-center justify-between border-b border-border/60 px-6 py-4">
              <div className="flex items-center gap-2 text-lg font-semibold text-foreground">
                <Clock className="h-4 w-4 text-primary" />
                Recently Active Agents
              </div>
              <button
                type="button"
                onClick={() => navigate('agents')}
                className="rounded-md border border-primary/20 bg-primary/5 px-2 py-1 text-xs font-semibold text-primary transition hover:bg-primary/10"
              >
                View all
              </button>
            </div>
            <div className="flex-1 divide-y divide-border/60">
              {statsLoading ? (
                <div className="flex h-full items-center justify-center py-12 text-sm text-muted-foreground">
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Loading…
                </div>
              ) : recentlyUsedAgents.length === 0 ? (
                <div className="flex h-full items-center justify-center py-12 text-center text-sm text-muted-foreground">
                  No recently used agents.
                </div>
              ) : (
                recentlyUsedAgents.slice(0, 4).map((agent, index) => {
                  const avatarStyle = agentAvatarStyles[index % agentAvatarStyles.length];
                  return (
                    <button
                      key={agent.id}
                      type="button"
                      onClick={() => {
                        const getPath = config.navigationPaths?.agentDetails;
                        const path = getPath
                          ? getPath(agent.id)
                          : `agents/${encodeURIComponent(agent.id)}`;
                        navigate(path);
                      }}
                      className="group flex w-full items-center gap-3 px-6 py-3.5 text-left transition hover:bg-muted/40"
                    >
                      <div
                        className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-full ${avatarStyle.bg} ${avatarStyle.text}`}
                      >
                        <Bot className="h-3.5 w-3.5" />
                      </div>
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-sm font-medium text-foreground group-hover:text-primary">
                          {agent.name}
                        </p>
                        <p className="mt-0.5 truncate text-xs text-muted-foreground">
                          {formatRelativeTime(agent.last_used_at)}
                        </p>
                      </div>
                    </button>
                  );
                })
              )}
            </div>
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

const agentAvatarStyles = [
  { bg: 'bg-indigo-100 dark:bg-indigo-500/20', text: 'text-indigo-600 dark:text-indigo-300' },
  { bg: 'bg-amber-100 dark:bg-amber-500/20', text: 'text-amber-600 dark:text-amber-300' },
  { bg: 'bg-rose-100 dark:bg-rose-500/20', text: 'text-rose-600 dark:text-rose-300' },
  { bg: 'bg-teal-100 dark:bg-teal-500/20', text: 'text-teal-600 dark:text-teal-300' },
  { bg: 'bg-cyan-100 dark:bg-cyan-500/20', text: 'text-cyan-600 dark:text-cyan-300' },
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
