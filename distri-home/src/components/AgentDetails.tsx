import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import { Chat, useAgent, useTheme, useDistri } from '@distri/react';
import { useDistriHomeNavigate, useDistriHome } from '../DistriHomeProvider';
import { useAgentValidation } from '../hooks/useAgentValidation';
import { CodePanel } from './CodePanel';
import {
  ArrowUpRight,
  CheckCircle2,
  ChevronRight,
  Clock,
  Copy,
  FileText,
  Globe,
  Loader2,
  MessageCircle,
  MessageSquare,
  Moon,
  AlertTriangle,
  Play,
  RefreshCw,
  Sun,
  Users,
  Wrench,
  XCircle,
  Plus,
} from 'lucide-react';
import { AgentConfigWithTools } from '@distri/core';
import { ToolDefinition } from '@distri/core';

const currentThreadId = (scope: string) => {
  if (typeof window === 'undefined') {
    return crypto.randomUUID();
  }
  const storageKey = `${scope}:threadId`;
  const cached = window.localStorage.getItem(storageKey);
  if (cached) return cached;
  const generated = crypto.randomUUID();
  window.localStorage.setItem(storageKey, generated);
  return generated;
};

const resetThreadId = (scope: string) => {
  const storageKey = `${scope}:threadId`;
  const generated = crypto.randomUUID();
  window.localStorage.setItem(storageKey, generated);
  return generated;
};

interface AgentDefinitionEnvelope extends AgentConfigWithTools {
  is_owner?: boolean;
  [key: string]: any;
}

export interface AgentDetailsProps {
  /**
   * The agent ID to display
   */
  agentId: string;
  /**
   * Optional thread ID to load in chat
   */
  threadId?: string;
  /**
   * Default tab to open
   */
  defaultTab?: 'definition' | 'chat' | 'tools' | 'integrate';
  /**
   * Optional custom class name
   */
  className?: string;
}

export function AgentDetails({
  agentId,
  threadId: propThreadId,
  defaultTab = 'definition',
  className,
}: AgentDetailsProps) {
  const navigate = useDistriHomeNavigate();
  const { config } = useDistriHome();
  const { client } = useDistri();
  const { agent, loading: agentLoading, error: agentError } = useAgent({ agentIdOrDef: agentId || '' });
  const { validation, warnings, loading: validationLoading, refetch: refetchValidation } = useAgentValidation({ agentId, enabled: !!agentId });
  const { theme, setTheme } = useTheme();
  const [isValidating, setIsValidating] = useState(false);
  const [showValidationPanel, setShowValidationPanel] = useState(false);

  const [definition, setDefinition] = useState<AgentDefinitionEnvelope | null>(null);
  const [sourceLoading, setSourceLoading] = useState(false);
  const [activePanel, setActivePanel] = useState<string>(
    defaultTab
  );
  const [activeSample, setActiveSample] = useState<'curl' | 'node' | 'python'>('curl');
  const [copied, setCopied] = useState(false);
  const [definitionDraft, setDefinitionDraft] = useState<string>('');
  const [savingDefinition, setSavingDefinition] = useState(false);
  const [definitionSaved, setDefinitionSaved] = useState(false);

  const [threadId, setThreadId] = useState(() => {
    if (propThreadId) return propThreadId;
    return currentThreadId(agentId ? `agent:${agentId}` : 'agent');
  });

  const handleNewChat = useCallback(() => {
    const newId = resetThreadId(agentId ? `agent:${agentId}` : 'agent');
    setThreadId(newId);
  }, [agentId]);

  const agentType = agent?.getDefinition?.().agent_type;

  const agentDefinition: AgentDefinitionEnvelope = useMemo(() => {
    if (definition) return definition;
    return agent?.getDefinition?.() as AgentDefinitionEnvelope;
  }, [agent, definition]);

  const toolDefinitions: ToolDefinition[] = useMemo(() => {
    return definition?.resolved_tools || agentDefinition?.resolved_tools || []
  }, [definition]);

  const toolRows = useMemo(() => {
    return toolDefinitions.map((tool: any) => {
      const name = tool?.name ?? tool?.function?.name ?? tool?.id ?? 'unknown_tool';
      const description =
        tool?.description ?? tool?.function?.description ?? tool?.metadata?.description ?? '';
      return { name, description };
    });
  }, [toolDefinitions]);

  const externalToolValidation = useMemo(() => {
    if (!agent) {
      return {
        isValid: true,
        requiredTools: [] as string[],
        providedTools: [] as string[],
        missingTools: [] as string[],
        message: undefined,
      };
    }
    return (agent as any).validateExternalTools?.() ?? {
      isValid: true,
      requiredTools: [],
      providedTools: [],
      missingTools: [],
    };
  }, [agent]);

  const isOwner = definition?.is_owner !== false;

  const hasExternalTools = externalToolValidation.requiredTools.length > 0;
  const chatDisabled = hasExternalTools;
  const embeddedAgentMessage =
    'Agent has external tools. This is an embedded Agent that can run within the parent application. Register DistriWidget for embedding the parent component.';

  useEffect(() => {
    if (!agentId || !client) {
      return;
    }
    const load = async () => {
      setSourceLoading(true);
      try {
        const data = await client.getAgent(agentId);
        setDefinition(data as AgentDefinitionEnvelope);
        setDefinitionDraft(data.markdown ?? '');
        setDefinitionSaved(false);
      } catch (err) {
        console.error(err);
      } finally {
        setSourceLoading(false);
      }
    };
    void load();
  }, [agentId, client]);

  const isDirty = definitionDraft !== (definition?.markdown ?? '');

  const handleResetDefinition = () => {
    setDefinitionDraft(definition?.markdown ?? '');
    setDefinitionSaved(false);
  };

  const handleSaveDefinition = async () => {
    if (!client || !agentId || !isDirty) return;
    setSavingDefinition(true);
    try {
      await client.updateAgent(agentId, { markdown: definitionDraft });
      // Reload to get updated definition
      const data = await client.getAgent(agentId);
      setDefinition(data as AgentDefinitionEnvelope);
      setDefinitionDraft(data.markdown ?? '');
      setDefinitionSaved(true);
    } catch (err) {
      console.error('Failed to save definition', err);
    } finally {
      setSavingDefinition(false);
    }
  };

  if (agentLoading || sourceLoading) {
    return (
      <div className={`flex h-full items-center justify-center bg-background ${className ?? ''}`}>
        <div className="flex items-center gap-3 text-muted-foreground">
          <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          Loading agent…
        </div>
      </div>
    );
  }

  // Handle errors specifically
  if (agentError) {
    return (
      <div className={`flex h-full items-center justify-center bg-background px-4 ${className ?? ''}`}>
        <div className="flex max-w-md flex-col items-center text-center gap-2">
          <p className="text-lg font-semibold text-destructive">Failed to load agent</p>
          <p className="text-sm text-muted-foreground">{agentError.message}</p>
          <button
            onClick={() => window.location.reload()}
            className="mt-4 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  if (!agent) {
    return (
      <div className={`flex h-full items-center justify-center bg-background px-4 ${className ?? ''}`}>
        <div className="flex max-w-md flex-col items-center text-center gap-2">
          <p className="text-lg font-semibold text-foreground">Agent not found</p>
          <p className="text-sm text-muted-foreground">
            Check the URL or verify that you have access to this agent.
          </p>
        </div>
      </div>
    );
  }

  const displayName = agentDefinition?.name ?? (agent as any)?.name ?? agentId ?? 'Agent';
  const description = agentDefinition?.description ?? (agent as any)?.description ?? '';
  const packageName = agentDefinition?.package_name;
  const version = agentDefinition?.version;
  const modelName = agentDefinition?.model_settings?.model;
  const analysisModelName = agentDefinition?.analysis_model_settings?.model;
  const maxIterations = agentDefinition?.max_iterations;
  const historySize = agentDefinition?.history_size;
  const contextSize = agentDefinition?.context_size ?? agentDefinition?.model_settings?.context_size;
  const browserEnabled = Boolean(agentDefinition?.browser_config?.enabled);
  const subAgents = Array.isArray(agentDefinition?.sub_agents) ? agentDefinition?.sub_agents : [];
  const skillCount = agentDefinition?.skills?.length ?? 0;
  const agentFilterId = agentDefinition?.id ?? agentId ?? displayName;
  const sampleAgentRef = agentDefinition?.id ?? agentId ?? 'agent_id';
  const sampleBaseUrl = client?.baseUrl ?? 'YOUR_API_URL';

  const sampleSnippets = {
    curl: [
      `curl -X POST "${sampleBaseUrl}/agents/${sampleAgentRef}/invoke" \\`,
      `  -H "Content-Type: application/json" \\`,
      `  -d '{ "input": "Hello, agent!" }'`,
    ].join('\n'),
    node: [
      `import fetch from 'node-fetch'`,
      ``,
      `const res = await fetch("${sampleBaseUrl}/agents/${sampleAgentRef}/invoke", {`,
      `  method: "POST",`,
      `  headers: { "Content-Type": "application/json" },`,
      `  body: JSON.stringify({ input: "Hello, agent!" })`,
      `})`,
      `const data = await res.json()`,
      `console.log(data)`,
    ].join('\n'),
    python: [
      `import requests`,
      ``,
      `resp = requests.post("${sampleBaseUrl}/agents/${sampleAgentRef}/invoke",`,
      `  json={"input": "Hello, agent!"})`,
      `print(resp.json())`,
    ].join('\n'),
  };

  const handleCopyDefinition = async () => {
    const markdown = definition?.markdown ?? '';
    if (!markdown) return;
    try {
      await navigator.clipboard.writeText(markdown);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error('Failed to copy definition', err);
    }
  };

  const handleValidate = async () => {
    setIsValidating(true);
    setShowValidationPanel(true);
    try {
      await refetchValidation();
    } finally {
      setIsValidating(false);
    }
  };

  const tabs = [
    { id: 'definition', label: 'Definition', icon: <FileText className="h-4 w-4" /> },
    { id: 'chat', label: 'Chat', icon: <MessageCircle className="h-4 w-4" /> },
    { id: 'tools', label: 'Tools', icon: <Wrench className="h-4 w-4" /> },
    { id: 'integrate', label: 'Integrate', icon: <Play className="h-4 w-4" /> },
    ...(config.customTabs || []).map(tab => ({
      id: tab.id,
      label: tab.label,
      icon: tab.icon
    }))
  ];

  // Add default "Embed" tab for OSS if no custom embed tab is provided
  const hasInjectedEmbed = (config.customTabs || []).some(t => t.id === 'embed');
  if (!hasInjectedEmbed) {
    tabs.push({
      id: 'embed_oss',
      label: 'Embed',
      icon: <div className="relative"><Globe className="h-4 w-4" /><AlertTriangle className="absolute -right-1.5 -top-1.5 h-2.5 w-2.5 text-amber-500" /></div>
    });
  }

  const sampleTabs = [
    { id: 'curl' as const, label: 'cURL' },
    { id: 'node' as const, label: 'Node' },
    { id: 'python' as const, label: 'Python' },
  ] as const;

  return (
    <div className={`flex-1 overflow-hidden bg-background ${className ?? ''}`}>
      <div className="mx-auto flex h-full min-h-0 w-full max-w-[1600px] flex-col px-6 py-6 lg:px-10">
        <header className="flex flex-wrap items-center justify-between gap-4 border-b border-border/60 pb-4">
          <div className="flex flex-wrap items-center gap-3">
            <nav className="flex items-center gap-2 text-sm text-muted-foreground">
              <button
                type="button"
                onClick={() => navigate('/agents')}
                className="hover:text-foreground"
              >
                Agents
              </button>
              <ChevronRight className="h-4 w-4 text-muted-foreground/70" />
              <span className="font-medium text-foreground">{displayName}</span>
            </nav>
            <span className="rounded-full border border-emerald-500/20 bg-emerald-500/10 px-2 py-0.5 text-xs font-semibold text-emerald-600 dark:text-emerald-300">
              Active
            </span>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => setTheme(theme === 'light' ? 'dark' : 'light')}
              className="flex h-9 w-9 items-center justify-center rounded-full border border-border/70 text-muted-foreground transition hover:text-foreground"
              title="Toggle theme"
            >
              {theme === 'light' ? <Moon className="h-4 w-4" /> : <Sun className="h-4 w-4" />}
            </button>
            <div className="h-6 w-px bg-border/70" />
            <button
              type="button"
              onClick={handleValidate}
              disabled={isValidating}
              className={`inline-flex items-center gap-2 rounded-md border px-3 py-1.5 text-sm font-medium transition ${validation?.valid === true && !isValidating
                ? 'border-emerald-500/50 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300 hover:bg-emerald-500/20'
                : validation?.valid === false && !isValidating
                  ? 'border-red-500/50 bg-red-500/10 text-red-700 dark:text-red-300 hover:bg-red-500/20'
                  : 'border-border/70 bg-card text-foreground hover:border-primary/50 hover:text-primary'
                } disabled:opacity-50`}
            >
              {isValidating ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : validation?.valid === true ? (
                <CheckCircle2 className="h-4 w-4" />
              ) : validation?.valid === false ? (
                <XCircle className="h-4 w-4" />
              ) : (
                <RefreshCw className="h-4 w-4" />
              )}
              {isValidating ? 'Validating…' : 'Validate'}
            </button>
            <button
              type="button"
              onClick={() => {
                const cloneTarget = definition?.id ?? agentId ?? agentDefinition?.name ?? displayName;
                navigate(`/home/new?clone_from_id=${encodeURIComponent(cloneTarget)}`);
              }}
              className="inline-flex items-center gap-2 rounded-md border border-border/70 bg-card px-3 py-1.5 text-sm font-medium text-foreground transition hover:border-primary/50 hover:text-primary"
            >
              <Copy className="h-4 w-4" />
              Clone
            </button>
          </div>
        </header>

        {/* Validation Panel */}
        {(showValidationPanel || warnings.length > 0) && !validationLoading && (
          <div className="mt-4 rounded-xl border border-border/70 bg-card overflow-hidden">
            <div className="flex items-center justify-between border-b border-border/60 bg-muted/40 px-4 py-3">
              <div className="flex items-center gap-2">
                {validation?.valid === true ? (
                  <>
                    <CheckCircle2 className="h-5 w-5 text-emerald-500" />
                    <span className="text-sm font-medium text-emerald-700 dark:text-emerald-300">
                      Validation Passed
                    </span>
                  </>
                ) : validation?.valid === false ? (
                  <>
                    <XCircle className="h-5 w-5 text-red-500" />
                    <span className="text-sm font-medium text-red-700 dark:text-red-300">
                      Validation Failed ({warnings.filter(w => w.severity === 'error').length} error{warnings.filter(w => w.severity === 'error').length !== 1 ? 's' : ''}, {warnings.filter(w => w.severity === 'warning').length} warning{warnings.filter(w => w.severity === 'warning').length !== 1 ? 's' : ''})
                    </span>
                  </>
                ) : warnings.length > 0 ? (
                  <>
                    <AlertTriangle className="h-5 w-5 text-amber-500" />
                    <span className="text-sm font-medium text-amber-700 dark:text-amber-300">
                      {warnings.length} Warning{warnings.length !== 1 ? 's' : ''}
                    </span>
                  </>
                ) : null}
              </div>
              <button
                type="button"
                onClick={() => setShowValidationPanel(false)}
                className="text-xs text-muted-foreground hover:text-foreground"
              >
                Dismiss
              </button>
            </div>

            {validation?.valid === true && warnings.length === 0 ? (
              <div className="p-4">
                <p className="text-sm text-muted-foreground">
                  All checks passed. Your agent is correctly configured and ready to use.
                </p>
              </div>
            ) : warnings.length > 0 ? (
              <div className="divide-y divide-border/60">
                {warnings.map((warning, index) => (
                  <div
                    key={`${warning.code}-${index}`}
                    className="flex items-start gap-3 px-4 py-3"
                  >
                    {warning.severity === 'error' ? (
                      <XCircle className="h-5 w-5 shrink-0 text-red-500 mt-0.5" />
                    ) : (
                      <AlertTriangle className="h-5 w-5 shrink-0 text-amber-500 mt-0.5" />
                    )}
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <p className={`text-sm font-medium ${warning.severity === 'error'
                          ? 'text-red-900 dark:text-red-100'
                          : 'text-amber-900 dark:text-amber-100'
                          }`}>
                          {warning.message}
                        </p>
                        <span className={`shrink-0 rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase ${warning.severity === 'error'
                          ? 'bg-red-500/20 text-red-700 dark:text-red-300'
                          : 'bg-amber-500/20 text-amber-700 dark:text-amber-300'
                          }`}>
                          {warning.severity}
                        </span>
                      </div>
                      <p className="mt-1 text-xs text-muted-foreground font-mono">
                        Code: {warning.code}
                      </p>
                      {warning.code === 'missing_provider_secret' && (
                        <button
                          type="button"
                          onClick={() => navigate('/settings/secrets')}
                          className="mt-2 inline-flex items-center gap-1 text-xs font-medium text-primary hover:text-primary/80"
                        >
                          Configure secrets <ArrowUpRight className="h-3 w-3" />
                        </button>
                      )}
                      {warning.code === 'missing_model_config' && (
                        <button
                          type="button"
                          onClick={() => navigate('/settings')}
                          className="mt-2 inline-flex items-center gap-1 text-xs font-medium text-primary hover:text-primary/80"
                        >
                          Configure model settings <ArrowUpRight className="h-3 w-3" />
                        </button>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            ) : null}
          </div>
        )}

        <div className="mt-4 flex flex-1 min-h-0 flex-col gap-6 xl:flex-row">
          <div className="flex min-h-0 flex-col gap-6 xl:flex-[5]">
            <div className="rounded-2xl border border-border/70 bg-card p-6 shadow-sm">
              <div className="flex flex-wrap items-start justify-between gap-4">
                <div className="space-y-2">
                  <p className="text-xs font-semibold uppercase tracking-[0.3em] text-muted-foreground">
                    Agent
                  </p>
                  <h1 className="text-2xl font-semibold text-foreground">{displayName}</h1>
                  <p className="text-sm text-muted-foreground">
                    {description || 'No description provided.'}
                  </p>
                  {/* Usage Stats */}
                  {agentDefinition?.stats && (
                    <div className="mt-3 flex flex-wrap items-center gap-4 text-sm">
                      <div className="flex items-center gap-1.5 text-muted-foreground">
                        <MessageSquare className="h-4 w-4" />
                        <span className="font-medium text-foreground">{agentDefinition.stats.thread_count}</span>
                        <span>thread{agentDefinition.stats.thread_count !== 1 ? 's' : ''}</span>
                      </div>
                      {agentDefinition.stats.sub_agent_usage_count > 0 && (
                        <div className="flex items-center gap-1.5 text-muted-foreground">
                          <Users className="h-4 w-4" />
                          <span className="font-medium text-foreground">{agentDefinition.stats.sub_agent_usage_count}</span>
                          <span>sub-agent call{agentDefinition.stats.sub_agent_usage_count !== 1 ? 's' : ''}</span>
                        </div>
                      )}
                      {agentDefinition.stats.last_used_at && (
                        <div className="flex items-center gap-1.5 text-muted-foreground">
                          <Clock className="h-4 w-4" />
                          <span>Last used {formatRelativeTime(agentDefinition.stats.last_used_at)}</span>
                        </div>
                      )}
                    </div>
                  )}
                  <button
                    type="button"
                    onClick={() => navigate(`/threads?agent=${encodeURIComponent(agentFilterId)}`)}
                    className="inline-flex items-center gap-1 text-sm font-medium text-primary hover:text-primary/80"
                  >
                    Recent threads <ArrowUpRight className="h-4 w-4" />
                  </button>
                </div>
                <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
                  <DetailBadge label="Type" value={formatAgentType(agentType)} />
                  {packageName ? <DetailBadge label="Package" value={packageName} /> : null}
                  {version ? <DetailBadge label="Version" value={version} /> : null}
                </div>
              </div>

              <div className="mt-6 grid gap-4 sm:grid-cols-2">
                <InfoBlock label="Agent ID" value={agentId || agentDefinition?.id || (agent as any)?.id || '—'} />
                <InfoBlock label="Version" value={String(version || '—')} />
              </div>
              <div className="mt-4 space-y-3">
                <DetailRow
                  label="Sub-agents"
                  value={
                    subAgents.length ? (
                      <div className="flex flex-wrap justify-end gap-2">
                        {subAgents.map((subAgent: string) => (
                          <button
                            key={subAgent}
                            type="button"
                            onClick={() => navigate(`/details?id=${encodeURIComponent(subAgent)}`)}
                            className="rounded-md border border-border/70 px-2 py-1 text-xs font-medium hover:border-primary/50 hover:text-primary"
                          >
                            {subAgent}
                          </button>
                        ))}
                      </div>
                    ) : (
                      'None'
                    )
                  }
                />
                <DetailRow label="Skills" value={skillCount ? String(skillCount) : 'None'} />
              </div>
            </div>

            <DetailCard title="Runtime configuration">
              <DetailRow label="Model" value={modelName || 'Default'} />
              <DetailRow label="Analysis model" value={analysisModelName || 'Default'} />
              <DetailRow label="Max iterations" value={maxIterations ?? 'Default'} />
              <DetailRow label="History size" value={historySize ?? 'Default'} />
              <DetailRow label="Context size" value={contextSize ?? 'Default'} />
              <DetailRow label="Browser" value={browserEnabled ? 'Enabled' : 'Disabled'} />
              <DetailRow
                label="Tools"
                value={
                  toolRows.length ? (
                    <button
                      type="button"
                      onClick={() => setActivePanel('tools')}
                      className="inline-flex items-center gap-1 text-sm font-medium text-primary hover:text-primary/80"
                    >
                      {toolRows.length} tools
                      <ChevronRight className="h-3 w-3" />
                    </button>
                  ) : (
                    'None'
                  )
                }
              />
            </DetailCard>
          </div>

          <div className="flex min-h-0 flex-1 flex-col gap-4 xl:flex-[7]">
            <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-2xl border border-border/70 bg-card shadow-sm">
              <div className="flex items-center justify-between border-b border-border/60 bg-muted/40 px-4 py-3">
                <div className="flex gap-1 rounded-lg bg-background/80 p-1">
                  {tabs.map((tab) => (
                    <button
                      key={tab.id}
                      type="button"
                      onClick={() => setActivePanel(tab.id)}
                      className={`flex items-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium transition ${activePanel === tab.id
                        ? 'bg-muted text-foreground'
                        : 'text-muted-foreground hover:text-foreground'
                        }`}
                    >
                      {tab.icon}
                      {tab.label}
                    </button>
                  ))}
                </div>
                <div className="flex items-center gap-2 text-xs text-muted-foreground">
                  {activePanel === 'definition' ? (
                    <button
                      type="button"
                      onClick={handleCopyDefinition}
                      className="flex items-center gap-1 rounded-md px-2 py-1 text-muted-foreground hover:text-foreground"
                    >
                      <Copy className="h-3 w-3" />
                      {copied ? 'Copied' : 'Copy'}
                    </button>
                  ) : null}
                </div>
              </div>

              {activePanel === 'definition' && (
                <div className="flex-1 min-h-0 overflow-hidden p-4">
                  <div className="flex h-full flex-col overflow-hidden rounded-xl border border-border/70 bg-background">
                    <div className="flex items-center justify-between border-b border-border/60 bg-muted/40 px-4 py-2 text-xs text-muted-foreground">
                      <div className="flex items-center gap-2">
                        <span>{isOwner ? 'Editable' : 'Read-only'}</span>
                        {isDirty && <span className="text-amber-600 dark:text-amber-400">• Unsaved</span>}
                        {definitionSaved && !isDirty && <span className="text-emerald-600 dark:text-emerald-400">✓ Saved</span>}
                      </div>
                      <div className="flex items-center gap-2">
                        <span>Markdown</span>
                        {isOwner && isDirty && (
                          <>
                            <button
                              type="button"
                              onClick={handleResetDefinition}
                              className="rounded px-2 py-0.5 text-xs hover:bg-muted"
                            >
                              Reset
                            </button>
                            <button
                              type="button"
                              onClick={handleSaveDefinition}
                              disabled={savingDefinition}
                              className="rounded bg-primary px-2 py-0.5 text-xs text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
                            >
                              {savingDefinition ? 'Saving…' : 'Save'}
                            </button>
                          </>
                        )}
                      </div>
                    </div>
                    <div className="flex-1 min-h-0 overflow-hidden">
                      <CodePanel
                        value={definitionDraft}
                        onChange={(value) => {
                          if (!isOwner) return;
                          setDefinitionDraft(value);
                          setDefinitionSaved(false);
                        }}
                        language="markdown"
                        readOnly={!isOwner}
                        theme={theme === 'light' ? 'light' : 'dark'}
                      />
                    </div>
                  </div>
                </div>
              )}

              {activePanel === 'chat' && (
                <div className="flex-1 min-h-0 overflow-hidden p-4">
                  <div className="flex h-full flex-col overflow-hidden rounded-xl border border-border/70 bg-background">
                    {chatDisabled ? (
                      <div className="flex h-full w-full items-center justify-center p-6">
                        <div className="max-w-md rounded-lg border border-amber-500/40 bg-amber-500/10 p-4 text-sm text-amber-900 dark:text-amber-100">
                          <p className="text-sm font-semibold text-amber-900 dark:text-amber-200">
                            Chat disabled
                          </p>
                          <p className="mt-2 text-sm text-amber-800/90 dark:text-amber-100/90">
                            {embeddedAgentMessage}
                          </p>
                          {externalToolValidation.requiredTools.length ? (
                            <div className="mt-3 flex flex-wrap gap-2 text-xs text-amber-900/90 dark:text-amber-100/90">
                              {externalToolValidation.requiredTools.map((tool: string) => (
                                <span
                                  key={tool}
                                  className="rounded-md border border-amber-500/40 bg-amber-500/10 px-2 py-1"
                                >
                                  {tool}
                                </span>
                              ))}
                            </div>
                          ) : null}
                        </div>
                      </div>
                    ) : (
                      <div className="flex flex-col h-full">
                        <div className="flex justify-end p-2 border-b border-gray-200 dark:border-gray-700">
                          <button
                            onClick={handleNewChat}
                            className="p-1.5 rounded-md hover:bg-gray-100 dark:hover:bg-gray-800 text-gray-500 dark:text-gray-400"
                            title="New chat"
                          >
                            <Plus className="h-4 w-4" />
                          </button>
                        </div>
                        <div className="flex-1 min-h-0">
                          <Chat
                            key={threadId}
                            agent={agent}
                            threadId={threadId}
                            enableHistory={true}
                            theme={theme === 'light' ? 'light' : theme === 'dark' ? 'dark' : 'auto'}
                          />
                        </div>
                      </div>
                    )}
                  </div>
                </div>
              )}

              {activePanel === 'tools' && (
                <div className="flex-1 min-h-0 overflow-hidden p-4">
                  <div className="flex h-full flex-col overflow-hidden rounded-xl border border-border/70 bg-background">
                    <div className="border-b border-border/60 px-4 py-2 text-xs text-muted-foreground">
                      Registered tools
                    </div>
                    <div className="flex-1 overflow-y-auto p-4">
                      {hasExternalTools ? (
                        <div className="mb-4 space-y-2">
                          <p className="text-[11px] uppercase tracking-[0.2em] text-muted-foreground">
                            External tools required
                          </p>
                          <div className="flex flex-wrap gap-2">
                            {externalToolValidation.requiredTools.map((tool: string) => (
                              <span
                                key={tool}
                                className="rounded-md border border-amber-500/40 bg-amber-500/10 px-2 py-1 text-xs text-amber-900 dark:text-amber-100"
                              >
                                {tool}
                              </span>
                            ))}
                          </div>
                        </div>
                      ) : null}

                      {toolRows.length ? (
                        <div className="overflow-hidden rounded-md border border-border/70">
                          <table className="w-full table-fixed text-left text-sm">
                            <colgroup>
                              <col className="w-[35%]" />
                              <col className="w-[65%]" />
                            </colgroup>
                            <thead className="bg-muted/60 text-muted-foreground">
                              <tr>
                                <th className="px-3 py-2 text-[11px] font-medium uppercase tracking-[0.2em]">
                                  Name
                                </th>
                                <th className="px-3 py-2 text-[11px] font-medium uppercase tracking-[0.2em]">
                                  Description
                                </th>
                              </tr>
                            </thead>
                            <tbody>
                              {toolRows.map((tool, index) => {
                                const isExternal = externalToolValidation.requiredTools.includes(
                                  tool.name
                                );
                                return (
                                  <tr key={`${tool.name}-${index}`} className="border-t border-border/70">
                                    <td className="px-3 py-2 text-foreground truncate">
                                      <div className="flex items-center gap-2">
                                        <span title={tool.name} className="truncate">
                                          {tool.name}
                                        </span>
                                        {isExternal && (
                                          <span title="External Tool"><Globe className="h-3 w-3 text-amber-500 shrink-0" /></span>
                                        )}
                                      </div>
                                    </td>
                                    <td
                                      className="px-3 py-2 text-muted-foreground truncate"
                                      title={tool.description || 'No description'}
                                    >
                                      {tool.description || 'No description'}
                                    </td>
                                  </tr>
                                );
                              })}
                            </tbody>
                          </table>
                        </div>
                      ) : (
                        <span className="text-sm text-muted-foreground">No tools registered.</span>
                      )}
                    </div>
                  </div>
                </div>
              )}

              {activePanel === 'integrate' && (
                <div className="flex-1 min-h-0 overflow-hidden p-4">
                  <div className="flex h-full flex-col rounded-xl border border-border/70 bg-background">
                    <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/60 px-4 py-3">
                      <div>
                        <p className="text-xs font-semibold uppercase tracking-[0.3em] text-muted-foreground">
                          Integrate
                        </p>
                        <p className="text-sm text-muted-foreground">
                          Call this agent from your stack.
                        </p>
                      </div>
                      <div className="flex gap-1 rounded-lg bg-muted/70 p-1">
                        {sampleTabs.map((tab) => (
                          <button
                            key={tab.id}
                            type="button"
                            onClick={() => setActiveSample(tab.id)}
                            className={`rounded-md px-3 py-1.5 text-sm font-medium transition ${activeSample === tab.id
                              ? 'bg-background text-foreground'
                              : 'text-muted-foreground hover:text-foreground'
                              }`}
                          >
                            {tab.label}
                          </button>
                        ))}
                      </div>
                    </div>
                    <div className="flex-1 flex flex-col min-h-0 overflow-hidden px-4 pb-4 pt-3">
                      <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground mb-3">
                        <span>Base URL: {sampleBaseUrl}</span>
                        <span className="rounded-md border border-border/70 bg-muted px-2 py-1">
                          Agent: {sampleAgentRef}
                        </span>
                      </div>
                      <div className="flex-1 min-h-[180px] overflow-hidden rounded-md border border-border/70">
                        <CodePanel
                          value={sampleSnippets[activeSample]}
                          language={activeSample === 'curl' ? 'shell' : activeSample === 'node' ? 'javascript' : 'python'}
                          readOnly
                          theme="dark"
                        />
                      </div>
                    </div>
                  </div>
                </div>
              )}

              {config.customTabs?.map((tab) =>
                activePanel === tab.id ? (
                  <div key={tab.id} className="flex-1 min-h-0 overflow-hidden p-4">
                    <div className="flex h-full flex-col overflow-hidden rounded-xl border border-border/70 bg-background p-6 overflow-auto">
                      {tab.render({ agentId: agentId || agentDefinition.id || '' })}
                    </div>
                  </div>
                ) : null
              )}

              {activePanel === 'embed_oss' && (
                <div className="flex-1 min-h-0 overflow-hidden p-4">
                  <div className="flex h-full flex-col items-center justify-center rounded-xl border border-border/70 bg-background p-8 text-center">
                    <div className="flex h-12 w-12 items-center justify-center rounded-full bg-amber-500/10 text-amber-600">
                      <AlertTriangle className="h-6 w-6" />
                    </div>
                    <h3 className="mt-4 text-lg font-semibold text-foreground">Cloud-only Feature</h3>
                    <p className="mt-2 max-w-sm text-sm text-muted-foreground">
                      Embed configuration is only available on Distri Cloud. This feature requires a secure managed backend for public client IDs and origin validation.
                    </p>
                    <div className="mt-6 flex flex-col gap-3">
                      <a
                        href="https://app.distri.dev"
                        target="_blank"
                        rel="noopener noreferrer"
                        className="inline-flex items-center justify-center rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
                      >
                        Try Distri Cloud
                      </a>
                      <p className="text-xs text-muted-foreground">
                        Securely embed agents in minutes withmanaged infrastructure.
                      </p>
                    </div>
                  </div>
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

type DetailCardProps = {
  title: string;
  children: ReactNode;
  className?: string;
};

const DetailCard = ({ title, children, className }: DetailCardProps) => {
  return (
    <div className={`rounded-2xl border border-border/70 bg-card p-6 shadow-sm ${className ?? ''}`}>
      <p className="text-xs font-semibold uppercase tracking-[0.3em] text-muted-foreground">{title}</p>
      <div className="mt-4 flex flex-col gap-3">{children}</div>
    </div>
  );
};

type DetailRowProps = {
  label: string;
  value?: ReactNode;
};

const DetailRow = ({ label, value }: DetailRowProps) => {
  return (
    <div className="flex items-start justify-between gap-3 text-sm">
      <span className="text-muted-foreground">{label}</span>
      <div className="text-right text-foreground break-all">{value ?? '—'}</div>
    </div>
  );
};

type DetailBadgeProps = {
  label: string;
  value: ReactNode;
};

const DetailBadge = ({ label, value }: DetailBadgeProps) => {
  return (
    <span className="rounded-md border border-border/70 bg-muted px-2 py-1 text-[11px] text-muted-foreground">
      {label}: <span className="text-foreground">{value}</span>
    </span>
  );
};

const InfoBlock = ({ label, value }: { label: string; value: ReactNode }) => {
  return (
    <div className="rounded-lg border border-border/70 bg-muted/60 p-3">
      <p className="text-[11px] font-semibold uppercase tracking-[0.2em] text-muted-foreground">{label}</p>
      <p className="mt-2 font-mono text-sm text-foreground">{value}</p>
    </div>
  );
};

const formatAgentType = (value?: string) => {
  if (!value) return 'Standard Agent';
  return value
    .replace(/_/g, ' ')
    .replace(/\b\w/g, (match) => match.toUpperCase());
};

const formatRelativeTime = (dateString: string) => {
  const date = new Date(dateString);
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
};


