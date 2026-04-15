import { useCallback, useEffect, useMemo, useState } from 'react';
import { Check, ChevronDown, Copy, Layers, Loader2, Mic, Plus, Settings, Star, Trash2, Volume2, Wrench, X } from 'lucide-react';
import { useDistriHomeClient } from '../DistriHomeProvider';
import type { Secret, CustomModelEntry, CustomProviderConfig, ModelProviderDefinition, Model, ModelCapability, ModelPricing, ProviderKeyDefinition, TtsVoiceInfo } from '../DistriHomeClient';
import { VoicePreviewDialog } from './VoicePreviewDialog';

type AgentSettingsTab = 'models' | 'providers';

export interface AgentSettingsViewProps {
  className?: string;
  /** Active tab from URL: 'models' (default) or 'providers' */
  activeTab?: 'models' | 'providers';
  /** Callback when tab changes -- consumer should update URL */
  onTabChange?: (tab: 'models' | 'providers') => void;
}

function CapabilityBadge({ type }: { type: ModelCapability }) {
  const styles: Record<ModelCapability, string> = {
    completion: 'bg-blue-500/10 text-blue-500',
    tts: 'bg-emerald-500/10 text-emerald-500',
    stt: 'bg-purple-500/10 text-purple-500',
  };
  const labels: Record<ModelCapability, string> = {
    completion: 'Completion',
    tts: 'TTS',
    stt: 'STT',
  };
  return (
    <span className={`inline-flex items-center rounded-full px-1.5 py-0.5 text-[10px] font-medium ${styles[type]}`}>
      {type === 'stt' && <Mic className="h-2.5 w-2.5 mr-0.5" />}
      {labels[type]}
    </span>
  );
}

function formatContextWindow(tokens?: number): string {
  if (!tokens) return '';
  if (tokens >= 1_000_000) {
    const val = tokens / 1_000_000;
    return val % 1 === 0 ? `${val}M` : `${val.toFixed(1)}M`;
  }
  if (tokens >= 1_000) {
    const val = tokens / 1_000;
    return val % 1 === 0 ? `${val}K` : `${val.toFixed(1)}K`;
  }
  return String(tokens);
}


export function AgentSettingsView({ className, activeTab: activeTabProp, onTabChange }: AgentSettingsViewProps) {
  const homeClient = useDistriHomeClient();

  const activeTab: AgentSettingsTab = activeTabProp ?? 'models';
  const setActiveTab = (tab: AgentSettingsTab) => {
    if (onTabChange) onTabChange(tab);
  };

  const [secrets, setSecrets] = useState<Secret[]>([]);
  const [providers, setProviders] = useState<ModelProviderDefinition[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [defaultModel, setDefaultModel] = useState<string>('');
  const [defaultTtsModel, setDefaultTtsModel] = useState<string>('');
  const [defaultSttModel, setDefaultSttModel] = useState<string>('');
  const [customModels, setCustomModels] = useState<CustomModelEntry[]>([]);
  const [saving, setSaving] = useState(false);
  const [saveSuccess, setSaveSuccess] = useState(false);

  const [fieldValues, setFieldValues] = useState<Record<string, string>>({});
  const [savingField, setSavingField] = useState<string | null>(null);
  const [expandedProviders, setExpandedProviders] = useState<Set<string>>(new Set());

  // Custom provider management
  const [showAddProvider, setShowAddProvider] = useState(false);
  const [newProviderName, setNewProviderName] = useState('');
  const [newProviderUrl, setNewProviderUrl] = useState('');
  const [newProviderKey, setNewProviderKey] = useState('');
  const [newProviderProjectId, setNewProviderProjectId] = useState('');

  // Custom providers stored in workspace settings
  const [customProviders, setCustomProviders] = useState<CustomProviderConfig[]>([]);

  // Auto-expand provider when navigating from Models tab Configure button
  const [autoExpandProvider, setAutoExpandProvider] = useState<string | null>(null);

  // Model selector dialog
  const [selectorDialogOpen, setSelectorDialogOpen] = useState(false);
  const [selectorDialogCapability, setSelectorDialogCapability] = useState<ModelCapability>('completion');

  // Voice preview dialog
  const [voiceDialogOpen, setVoiceDialogOpen] = useState(false);
  const [voiceDialogModelId, setVoiceDialogModelId] = useState<string | undefined>();
  const [voiceDialogProviderId, setVoiceDialogProviderId] = useState<string | undefined>();

  const openVoiceDialog = useCallback((providerId: string, modelId: string) => {
    setVoiceDialogProviderId(providerId);
    setVoiceDialogModelId(modelId);
    setVoiceDialogOpen(true);
  }, []);

  const [copiedId, setCopiedId] = useState<string | null>(null);
  const handleCopyId = useCallback((id: string) => {
    navigator.clipboard.writeText(id).then(() => {
      setCopiedId(id);
      setTimeout(() => setCopiedId(null), 1500);
    });
  }, []);

  // Models tab filters

  // Inline add model: tracks which provider+section is adding (e.g. "openai:completion")
  const [addingModelKey, setAddingModelKey] = useState<string | null>(null);
  const [newModelId, setNewModelId] = useState<string>('');

  const loadData = useCallback(async () => {
    if (!homeClient) return;
    setLoading(true);
    setError(null);
    try {
      const [settings, secs, providerData] = await Promise.all([
        homeClient.getWorkspaceSettings(),
        homeClient.listSecrets(),
        homeClient.listProviders(),
      ]);
      setSecrets(secs);
      setProviders(providerData);
      setDefaultModel(settings?.default_model || '');
      setDefaultTtsModel(settings?.default_tts_model || '');
      setDefaultSttModel(settings?.default_stt_model || '');
      setCustomModels(settings?.custom_models || []);
      setCustomProviders(settings?.custom_providers || []);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load settings');
    } finally {
      setLoading(false);
    }
  }, [homeClient]);

  useEffect(() => { loadData(); }, [loadData]);

  const getSecret = useCallback(
    (key: string) => secrets.find((s) => s.key === key),
    [secrets],
  );

  const isProviderConfigured = useCallback(
    (provider: ModelProviderDefinition) => {
      const required = provider.keys.filter((k) => k.required !== false);
      if (required.length === 0) {
        // Provider with no required keys: configured if ANY key is present
        return provider.keys.some((k) => {
          const s = getSecret(k.key);
          return s && s.masked_value && s.masked_value !== '';
        });
      }
      return required.every((k) => {
        const s = getSecret(k.key);
        return s && s.masked_value && s.masked_value !== '';
      });
    },
    [getSecret],
  );

  const getProviderCapabilities = useCallback(
    (provider: ModelProviderDefinition): ModelCapability[] => {
      const caps = new Set<ModelCapability>();
      for (const m of provider.models) {
        caps.add(m.capability);
      }
      return Array.from(caps);
    },
    [],
  );

  // Save all unsaved fields for a provider at once via POST /providers
  const handleSaveProvider = async (providerId: string, keys: ProviderKeyDefinition[]) => {
    if (!homeClient) return;
    const toSave = keys.filter((k) => {
      const val = fieldValues[k.key]?.trim();
      return val && !getSecret(k.key);
    });
    if (toSave.length === 0) return;
    setSavingField('__provider__');
    setError(null);
    try {
      const secrets: Record<string, string> = {};
      for (const k of toSave) {
        secrets[k.key] = fieldValues[k.key].trim();
      }
      await homeClient.upsertProvider({ provider_id: providerId, secrets });
      setFieldValues((prev) => {
        const next = { ...prev };
        for (const k of toSave) delete next[k.key];
        return next;
      });
      await loadData();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save');
    } finally {
      setSavingField(null);
    }
  };

  const handleDeleteField = async (key: string) => {
    if (!homeClient) return;
    setError(null);
    try {
      await homeClient.deleteSecret(key);
      await loadData();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to delete');
    }
  };

  const handleSetDefault = async (type: ModelCapability, model: string) => {
    if (!homeClient) return;
    setSaving(true);
    try {
      if (type === 'completion') {
        const newDefault = defaultModel === model ? '' : model;
        setDefaultModel(newDefault);
        await homeClient.upsertProvider({
          provider_id: '__settings__',
          default_model: newDefault,
        });
      } else if (type === 'tts') {
        const newDefault = defaultTtsModel === model ? '' : model;
        setDefaultTtsModel(newDefault);
        await homeClient.updateWorkspaceSettings({ default_tts_model: newDefault || null });
      } else if (type === 'stt') {
        const newDefault = defaultSttModel === model ? '' : model;
        setDefaultSttModel(newDefault);
        await homeClient.updateWorkspaceSettings({ default_stt_model: newDefault || null });
      }
      setSaveSuccess(true);
      setTimeout(() => setSaveSuccess(false), 2000);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save');
    } finally {
      setSaving(false);
    }
  };

  const handleAddModel = async (providerId: string, capability: ModelCapability) => {
    if (!homeClient) return;
    const modelName = newModelId.trim();
    if (!modelName) return;

    if (customModels.some((m) => m.provider === providerId && m.model === modelName)) return;

    const updated = [...customModels, { provider: providerId, model: modelName, capability }];
    setCustomModels(updated);
    setNewModelId('');
    setAddingModelKey(null);
    try {
      await homeClient.upsertProvider({
        provider_id: '__settings__',
        custom_models: updated,
      });
      await loadData();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save');
    }
  };

  const handleRemoveCustomModel = async (providerId: string, modelName: string) => {
    if (!homeClient) return;
    const fullId = `${providerId}/${modelName}`;
    const updated = customModels.filter((m) => !(m.provider === providerId && m.model === modelName));
    setCustomModels(updated);

    let newDefaultModel = defaultModel;
    let newDefaultTts = defaultTtsModel;
    let newDefaultStt = defaultSttModel;
    if (defaultModel === fullId) { newDefaultModel = ''; setDefaultModel(''); }
    if (defaultTtsModel === fullId) { newDefaultTts = ''; setDefaultTtsModel(''); }
    if (defaultSttModel === fullId) { newDefaultStt = ''; setDefaultSttModel(''); }

    try {
      await homeClient.upsertProvider({
        provider_id: '__settings__',
        custom_models: updated,
        default_model: newDefaultModel,
      });
      if (newDefaultTts !== defaultTtsModel || newDefaultStt !== defaultSttModel) {
        await homeClient.updateWorkspaceSettings({
          default_tts_model: newDefaultTts || null,
          default_stt_model: newDefaultStt || null,
        });
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save');
    }
  };

  const handleAddCustomProvider = async () => {
    const name = newProviderName.trim();
    const url = newProviderUrl.trim();
    const key = newProviderKey.trim();
    if (!name || !url || !key || !homeClient) return;

    const id = `custom_${name.toLowerCase().replace(/[^a-z0-9]/g, '_')}`;

    setSaving(true);
    try {
      const secrets: Record<string, string> = {
        [`${id.toUpperCase()}_API_KEY`]: key,
      };
      if (newProviderProjectId.trim()) {
        secrets[`${id.toUpperCase()}_PROJECT_ID`] = newProviderProjectId.trim();
      }

      await homeClient.upsertProvider({
        provider_id: id,
        secrets,
        config: {
          id,
          name,
          base_url: url,
          ...(newProviderProjectId.trim() ? { project_id: newProviderProjectId.trim() } : {}),
        },
      });

      setNewProviderName('');
      setNewProviderUrl('');
      setNewProviderKey('');
      setNewProviderProjectId('');
      setShowAddProvider(false);
      await loadData();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save provider');
    } finally {
      setSaving(false);
    }
  };

  const handleRemoveCustomProvider = async (providerId: string) => {
    if (!homeClient) return;
    setError(null);
    try {
      await homeClient.deleteProvider(providerId);
      await loadData();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to delete provider');
    }
  };

  const toggleProvider = (id: string) => {
    setExpandedProviders((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  // Auto-expand provider when switching to providers tab with autoExpandProvider set
  useEffect(() => {
    if (activeTab === 'providers' && autoExpandProvider) {
      setExpandedProviders((prev) => {
        const next = new Set(prev);
        next.add(autoExpandProvider);
        return next;
      });
      setAutoExpandProvider(null);
    }
  }, [activeTab, autoExpandProvider]);

  // Build flat model list for the Models tab
  const allModels = useMemo(() => {
    const rows: ModelRow[] = [];

    for (const provider of providers) {
      const configured = isProviderConfigured(provider);

      for (const m of provider.models) {
        rows.push({
          id: m.id,
          name: m.name,
          providerId: provider.id,
          providerLabel: provider.label,
          type: m.capability,
          isCustom: false,
          providerConfigured: configured,
          contextWindow: m.context_window,
          pricing: m.pricing,
          voices: m.voices,
        });
      }
    }

    // Add custom models
    for (const cm of customModels) {
      if (rows.some((r) => r.providerId === cm.provider && r.id === cm.model)) continue;
      const provider = providers.find((p) => p.id === cm.provider);
      const configured = provider ? isProviderConfigured(provider) : false;
      const cap = (cm as any).capability as ModelCapability | undefined;
      rows.push({
        id: cm.model,
        name: cm.model,
        providerId: cm.provider,
        providerLabel: provider?.label || cm.provider,
        type: cap || 'completion',
        isCustom: true,
        providerConfigured: configured,
      });
    }

    return rows;
  }, [providers, customModels, isProviderConfigured]);

  // Group all models by provider
  const groupedModels = useMemo(() => {
    const groups: Array<{ providerId: string; providerLabel: string; models: ModelRow[] }> = [];
    const map = new Map<string, ModelRow[]>();
    const order: string[] = [];

    for (const m of allModels) {
      if (!map.has(m.providerId)) {
        map.set(m.providerId, []);
        order.push(m.providerId);
      }
      map.get(m.providerId)!.push(m);
    }

    for (const pid of order) {
      const models = map.get(pid)!;
      groups.push({
        providerId: pid,
        providerLabel: models[0].providerLabel,
        models,
      });
    }

    return groups;
  }, [allModels]);


  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
      </div>
    );
  }

  const getDefaultForType = (type: ModelCapability) => {
    if (type === 'completion') return defaultModel;
    if (type === 'tts') return defaultTtsModel;
    return defaultSttModel;
  };

  // Render a model table for a capability section
  const renderModelTable = (
    section: 'completion' | 'voice',
    title: string,
    groups: Array<{ providerId: string; providerLabel: string; models: ModelRow[] }>,
    headerAction?: React.ReactNode,
  ) => {
    const capTypes = section === 'completion' ? ['completion'] : ['tts', 'stt'];
    const filtered = groups
      .map(g => ({ ...g, models: g.models.filter(m => capTypes.includes(m.type)) }))
      .filter(g => g.models.length > 0);

    if (filtered.length === 0) return null;

    const isCompletion = section === 'completion';
    const configuredGroups = filtered.filter(g => g.models.some(m => m.providerConfigured));
    const unconfiguredGroups = filtered.filter(g => !g.models.some(m => m.providerConfigured));

    const renderTableHeader = () => isCompletion ? (
      <div className="grid grid-cols-[1fr_70px_120px_70px_36px] gap-3 px-6 py-2.5 border-b border-border/40 bg-muted/30">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">Model</span>
        <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground text-right">Context</span>
        <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground text-right">Cost / 1M</span>
        <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground text-right">Cached</span>
        <span />
      </div>
    ) : (
      <div className="grid grid-cols-[1fr_70px_130px_36px] gap-3 px-6 py-2.5 border-b border-border/40 bg-muted/30">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">Model</span>
        <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">Type</span>
        <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground text-right">Cost</span>
        <span />
      </div>
    );

    const renderModelRows = (models: ModelRow[]) => models.map((model) => {
      const fullId = `${model.providerId}/${model.id}`;
      const isDefault = getDefaultForType(model.type) === fullId;
      return isCompletion ? (
        <div key={`${model.providerId}-${model.id}`}
          className={`group grid grid-cols-[1fr_70px_120px_70px_36px] gap-3 items-start pl-10 pr-6 py-2.5 border-b border-border/10 last:border-b-0 transition ${model.providerConfigured ? 'hover:bg-muted/20' : 'opacity-40'}`}>
          <div className="flex flex-col gap-0.5 min-w-0">
            <div className="flex items-center gap-2 min-w-0">
              <span className="text-sm truncate text-muted-foreground">{model.name}</span>
              {model.isCustom && <span className="text-[9px] text-muted-foreground/40 bg-muted/40 px-1 py-0.5 rounded shrink-0">custom</span>}
              {model.isCustom && <button type="button" onClick={() => handleRemoveCustomModel(model.providerId, model.id)} className="text-muted-foreground/40 hover:text-destructive transition p-0.5 shrink-0" title="Remove"><X className="h-3.5 w-3.5" /></button>}
            </div>
            <div className="flex items-center gap-1">
              <span className="text-[11px] font-mono text-muted-foreground/40 truncate">{model.providerId}/{model.id}</span>
              <button type="button" onClick={() => handleCopyId(fullId)} className="text-muted-foreground/30 hover:text-muted-foreground transition shrink-0" title="Copy model ID">
                {copiedId === fullId ? <Check className="h-3 w-3 text-emerald-500" /> : <Copy className="h-3 w-3" />}
              </button>
            </div>
          </div>
          <div className="text-right">{model.contextWindow ? <span className="text-xs font-mono text-muted-foreground/60">{formatContextWindow(model.contextWindow)}</span> : <span className="text-xs text-muted-foreground/20">&mdash;</span>}</div>
          <div className="text-right">{model.pricing?.type === 'completion' ? <span className="text-xs font-mono text-muted-foreground/60">${model.pricing.input.toFixed(2)} / ${model.pricing.output.toFixed(2)}</span> : <span className="text-xs text-muted-foreground/20">&mdash;</span>}</div>
          <div className="text-right">{model.pricing?.type === 'completion' && model.pricing.cached_input != null ? <span className="text-xs font-mono text-muted-foreground/60">${model.pricing.cached_input.toFixed(2)}</span> : <span className="text-xs text-muted-foreground/20">&mdash;</span>}</div>
          <div className="flex justify-end"><button type="button" onClick={() => model.providerConfigured && handleSetDefault(model.type, fullId)} disabled={saving || !model.providerConfigured} className={`p-1 rounded transition ${isDefault ? 'text-primary' : model.providerConfigured ? 'text-muted-foreground/40 hover:text-primary' : 'opacity-0'}`} title={isDefault ? 'Default' : 'Set as default'}><Star className={`h-4 w-4 ${isDefault ? 'fill-primary' : ''}`} /></button></div>
        </div>
      ) : (
        <div key={`${model.providerId}-${model.type}-${model.id}`}
          className={`group grid grid-cols-[1fr_70px_130px_36px] gap-3 items-start pl-10 pr-6 py-2.5 border-b border-border/10 last:border-b-0 transition ${model.providerConfigured ? 'hover:bg-muted/20' : 'opacity-40'}`}>
          <button
            type="button"
            disabled={!model.providerConfigured || !model.voices?.length}
            onClick={() => model.type === 'tts' && model.voices?.length && openVoiceDialog(model.providerId, model.id)}
            className={`flex flex-col gap-0.5 min-w-0 text-left ${model.type === 'tts' && model.voices?.length && model.providerConfigured ? 'cursor-pointer' : 'cursor-default'}`}
            title={model.type === 'tts' && model.voices?.length ? `${model.voices.length} voices — click to preview` : undefined}
          >
            <div className="flex items-center gap-2 min-w-0">
              <span className="text-sm truncate text-muted-foreground">{model.name}</span>
              {model.type === 'tts' && model.voices?.length && <span className="text-[10px] text-muted-foreground/40 shrink-0">{model.voices.length} voices</span>}
              {model.isCustom && <span className="text-[9px] text-muted-foreground/40 bg-muted/40 px-1 py-0.5 rounded shrink-0">custom</span>}
              {model.isCustom && <button type="button" onClick={(e) => { e.stopPropagation(); handleRemoveCustomModel(model.providerId, model.id); }} className="text-muted-foreground/40 hover:text-destructive transition p-0.5 shrink-0" title="Remove"><X className="h-3.5 w-3.5" /></button>}
            </div>
            <div className="flex items-center gap-1">
              <span className="text-[11px] font-mono text-muted-foreground/40 truncate">{model.providerId}/{model.id}</span>
              <button type="button" onClick={(e) => { e.stopPropagation(); handleCopyId(fullId); }} className="text-muted-foreground/30 hover:text-muted-foreground transition shrink-0" title="Copy model ID">
                {copiedId === fullId ? <Check className="h-3 w-3 text-emerald-500" /> : <Copy className="h-3 w-3" />}
              </button>
            </div>
          </button>
          <div><CapabilityBadge type={model.type} /></div>
          <div className="text-right">{model.pricing?.type === 'tts' ? <span className="text-xs font-mono text-muted-foreground/60">${model.pricing.per_1m_chars.toFixed(2)} / 1M chars</span> : model.pricing?.type === 'stt' ? <span className="text-xs font-mono text-muted-foreground/60">${model.pricing.per_minute.toFixed(3)} / min</span> : <span className="text-xs text-muted-foreground/20">&mdash;</span>}</div>
          <div className="flex justify-end"><button type="button" onClick={() => model.providerConfigured && handleSetDefault(model.type, fullId)} disabled={saving || !model.providerConfigured} className={`p-1 rounded transition ${isDefault ? 'text-primary' : model.providerConfigured ? 'text-muted-foreground/40 hover:text-primary' : 'opacity-0'}`} title={isDefault ? 'Default' : 'Set as default'}><Star className={`h-4 w-4 ${isDefault ? 'fill-primary' : ''}`} /></button></div>
        </div>
      );
    });

    return (
      <div className="space-y-0">
        {(title || headerAction) && (
          <div className="flex items-center justify-between mb-2">
            {title && <p className="text-xs font-semibold uppercase tracking-[0.2em] text-muted-foreground">{title}</p>}
            <div className="flex-1" />
            {headerAction}
          </div>
        )}
        <div className="rounded-xl border border-border/70 bg-card shadow-sm overflow-hidden">
          {renderTableHeader()}
          {filtered.map((group) => {
            const isConfigured = group.models.some(m => m.providerConfigured);
            return (
              <details key={group.providerId} open={isConfigured || undefined}>
                <summary className="flex items-center gap-3 px-6 py-2.5 bg-muted/20 cursor-pointer hover:bg-muted/30 transition select-none list-none [&::-webkit-details-marker]:hidden">
                  <ChevronDown className="h-4 w-4 text-muted-foreground/50 transition-transform [[open]>&]:rotate-180 shrink-0" />
                  <span className={`text-[11px] font-semibold uppercase tracking-[0.15em] shrink-0 ${isConfigured ? 'text-muted-foreground/70' : 'text-muted-foreground/50'}`}>{group.providerLabel}</span>
                  <span className="text-[11px] text-muted-foreground/40">{group.models.length} model{group.models.length !== 1 ? 's' : ''}</span>
                  <div className="flex-1" />
                  <button type="button" onClick={(e) => {
                    e.preventDefault();
                    const key = `${group.providerId}:${section === 'completion' ? 'completion' : 'tts'}`;
                    setAddingModelKey(addingModelKey === key ? null : key);
                    setNewModelId('');
                  }} className="text-muted-foreground/50 hover:text-foreground transition p-1 shrink-0" title="Add model">
                    <Plus className="h-4 w-4" />
                  </button>
                  <button type="button" onClick={(e) => { e.preventDefault(); setAutoExpandProvider(group.providerId); onTabChange?.('providers'); }}
                    className="flex items-center gap-1.5 text-[11px] font-medium text-muted-foreground/60 hover:text-foreground transition shrink-0" title="Configure provider">
                    {isConfigured ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Wrench className="h-3.5 w-3.5" />}
                    Configure
                  </button>
                </summary>
                <div className={isConfigured ? '' : 'opacity-50'}>
                  {renderModelRows(group.models)}
                  {addingModelKey === `${group.providerId}:${section === 'completion' ? 'completion' : 'tts'}` && (
                    <div className="flex items-center gap-2 pl-10 pr-6 py-1.5 bg-muted/10 border-t border-border/20">
                      <input
                        type="text"
                        value={newModelId}
                        onChange={(e) => setNewModelId(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') handleAddModel(group.providerId, section === 'completion' ? 'completion' : 'tts');
                          if (e.key === 'Escape') { setAddingModelKey(null); setNewModelId(''); }
                        }}
                        placeholder="Model ID (e.g. gpt-4o-mini)"
                        autoFocus
                        className="flex-1 rounded border border-border/50 bg-background px-2 py-1 text-xs text-foreground placeholder:text-muted-foreground/40 focus:border-primary focus:outline-none"
                      />
                      {section === 'voice' && (
                        <select
                          value={addingModelKey?.split(':')[1] || 'tts'}
                          onChange={(e) => setAddingModelKey(`${group.providerId}:${e.target.value}`)}
                          className="rounded border border-border/50 bg-background px-2 py-1 text-xs text-foreground"
                        >
                          <option value="tts">TTS</option>
                          <option value="stt">STT</option>
                        </select>
                      )}
                      <button type="button" onClick={() => handleAddModel(group.providerId, (addingModelKey?.split(':')[1] || 'completion') as ModelCapability)}
                        disabled={!newModelId.trim()} className="text-xs text-primary hover:text-primary/80 disabled:opacity-30">Add</button>
                      <button type="button" onClick={() => { setAddingModelKey(null); setNewModelId(''); }}
                        className="text-muted-foreground/40 hover:text-foreground"><X className="h-3 w-3" /></button>
                    </div>
                  )}
                </div>
              </details>
            );
          })}
        </div>
      </div>
    );
  };

  // Render a provider card for the Providers tab
  const renderProviderCard = (provider: ModelProviderDefinition) => {
    const configured = isProviderConfigured(provider);
    const isExpanded = expandedProviders.has(provider.id);
    const capabilities = getProviderCapabilities(provider);

    return (
      <div key={provider.id} className="rounded-xl border border-border/70 bg-card shadow-sm overflow-hidden">
        {/* Header */}
        <button
          type="button"
          onClick={() => toggleProvider(provider.id)}
          className="flex w-full items-center justify-between px-6 py-4 text-left hover:bg-muted/30 transition"
        >
          <div className="flex items-center gap-3">
            <div>
              <h3 className="text-sm font-semibold text-foreground">{provider.label}</h3>
              {!isExpanded && (
                <p className="text-xs text-muted-foreground mt-0.5">
                  {configured ? 'Configured' : 'Not configured'}
                </p>
              )}
            </div>
            <div className="flex items-center gap-1">
              {provider.is_custom && (
                <span className="text-[10px] font-medium px-1.5 py-0.5 rounded-full bg-amber-500/10 text-amber-500">Custom</span>
              )}
              {capabilities.map((cap) => (
                <CapabilityBadge key={cap} type={cap} />
              ))}
            </div>
          </div>
          <div className="flex items-center gap-2">
            {provider.is_custom && (
              <span
                role="button"
                onClick={(e) => { e.stopPropagation(); handleRemoveCustomProvider(provider.id); }}
                className="text-muted-foreground/40 hover:text-destructive transition p-1"
                title="Remove provider"
              >
                <Trash2 className="h-3.5 w-3.5" />
              </span>
            )}
            {configured && (
              <div className="flex h-6 w-6 items-center justify-center rounded-full bg-emerald-500/20 text-emerald-500 shrink-0">
                <Check className="h-3.5 w-3.5" />
              </div>
            )}
            <ChevronDown className={`h-4 w-4 text-muted-foreground transition-transform ${isExpanded ? 'rotate-180' : ''}`} />
          </div>
        </button>

        {/* Expanded body */}
        {isExpanded && <>
          {/* Secret key fields */}
          {provider.keys.length > 0 && (() => {
            const hasUnsaved = provider.keys.some((k) => !getSecret(k.key) && fieldValues[k.key]?.trim());
            return (
              <div className="px-6 py-4 space-y-3">
                {provider.keys.map((keyDef) => {
                  const isSensitive = keyDef.sensitive !== false;
                  const existing = getSecret(keyDef.key);

                  return (
                    <div key={keyDef.key}>
                      <div className="flex items-center justify-between mb-1.5">
                        <label className="text-xs font-medium text-muted-foreground">
                          {keyDef.label}
                          {!keyDef.required && <span className="ml-1 text-muted-foreground/40">(optional)</span>}
                        </label>
                        {existing && (
                          <button
                            type="button"
                            onClick={() => handleDeleteField(existing.key)}
                            className="text-muted-foreground/40 transition hover:text-destructive"
                            title="Remove"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </button>
                        )}
                      </div>
                      {existing ? (
                        <div className="rounded-lg border border-border/50 bg-muted/30 px-3 py-2 text-sm font-mono text-foreground">
                          {isSensitive ? '\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022' : existing.masked_value}
                        </div>
                      ) : (
                        <input
                          type={isSensitive ? 'password' : 'text'}
                          value={fieldValues[keyDef.key] || ''}
                          onChange={(e) =>
                            setFieldValues((prev) => ({ ...prev, [keyDef.key]: e.target.value }))
                          }
                          placeholder={keyDef.placeholder}
                          className="w-full rounded-lg border border-border/70 bg-background px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground/40 focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                        />
                      )}
                    </div>
                  );
                })}
                {/* Provider-level save */}
                {hasUnsaved && (
                  <div className="flex justify-end pt-1">
                    <button
                      type="button"
                      onClick={() => handleSaveProvider(provider.id, provider.keys)}
                      disabled={savingField === '__provider__'}
                      className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition hover:bg-primary/90 disabled:opacity-50"
                    >
                      {savingField === '__provider__' ? 'Saving...' : 'Save'}
                    </button>
                  </div>
                )}
              </div>
            );
          })()}
        </>}
      </div>
    );
  };

  return (
    <div className={`space-y-6 ${className ?? ''}`}>
      {error && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}
      {saveSuccess && (
        <div className="rounded-md border border-emerald-500/40 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-600">
          Settings saved.
        </div>
      )}

      {/* Tab Bar */}
      <div className="flex gap-1 rounded-lg bg-muted/50 p-1">
        <button
          type="button"
          onClick={() => setActiveTab('models')}
          className={`flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium transition ${
            activeTab === 'models'
              ? 'bg-background text-foreground shadow-sm'
              : 'text-muted-foreground hover:text-foreground'
          }`}
        >
          <Layers className="h-3.5 w-3.5" />
          Models
        </button>
        <button
          type="button"
          onClick={() => setActiveTab('providers')}
          className={`flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium transition ${
            activeTab === 'providers'
              ? 'bg-background text-foreground shadow-sm'
              : 'text-muted-foreground hover:text-foreground'
          }`}
        >
          <Settings className="h-3.5 w-3.5" />
          Providers
        </button>
      </div>

      {/* Providers Tab */}
      {activeTab === 'providers' && (<>
        {[...providers].sort((a, b) => {
          const aConfigured = isProviderConfigured(a) ? 0 : 1;
          const bConfigured = isProviderConfigured(b) ? 0 : 1;
          return aConfigured - bConfigured;
        }).map((provider) => renderProviderCard(provider))}

        {/* Add custom provider */}
        <div className="rounded-xl border border-dashed border-border/70 bg-card/50 shadow-sm overflow-hidden">
          {showAddProvider ? (
            <div className="px-6 py-4 space-y-3">
              <div className="flex items-center justify-between">
                <h3 className="text-sm font-semibold text-foreground">Add Custom Provider</h3>
                <button type="button" onClick={() => setShowAddProvider(false)} className="text-muted-foreground hover:text-foreground">
                  <X className="h-4 w-4" />
                </button>
              </div>
              <p className="text-xs text-muted-foreground">OpenAI-compatible endpoint (vLLM, LiteLLM, LangDB, Ollama, etc).</p>
              <div>
                <label className="text-xs font-medium text-muted-foreground mb-1 block">Provider Name</label>
                <input type="text" value={newProviderName} onChange={(e) => setNewProviderName(e.target.value)} placeholder="e.g. LangDB, My Ollama, Company LLM" className="w-full rounded-lg border border-border/70 bg-background px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground/40 focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary" />
              </div>
              <div>
                <label className="text-xs font-medium text-muted-foreground mb-1 block">API URL</label>
                <input type="text" value={newProviderUrl} onChange={(e) => setNewProviderUrl(e.target.value)} placeholder="https://api.example.com/v1" className="w-full rounded-lg border border-border/70 bg-background px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground/40 focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary" />
              </div>
              <div>
                <label className="text-xs font-medium text-muted-foreground mb-1 block">API Key</label>
                <input type="password" value={newProviderKey} onChange={(e) => setNewProviderKey(e.target.value)} placeholder="sk-..." className="w-full rounded-lg border border-border/70 bg-background px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground/40 focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary" />
              </div>
              <div>
                <label className="text-xs font-medium text-muted-foreground mb-1 block">Project ID <span className="text-muted-foreground/40">(optional)</span></label>
                <input type="text" value={newProviderProjectId} onChange={(e) => setNewProviderProjectId(e.target.value)} placeholder="project-123" className="w-full rounded-lg border border-border/70 bg-background px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground/40 focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary" />
              </div>
              <button type="button" onClick={handleAddCustomProvider} disabled={!newProviderName.trim() || !newProviderUrl.trim() || !newProviderKey.trim() || saving} className="w-full rounded-lg bg-primary py-2 text-sm font-medium text-primary-foreground transition hover:bg-primary/90 disabled:opacity-50">
                {saving ? 'Adding...' : 'Add Provider'}
              </button>
            </div>
          ) : (
            <button type="button" onClick={() => setShowAddProvider(true)} className="flex w-full items-center justify-center gap-2 px-6 py-4 text-sm text-muted-foreground transition hover:text-foreground">
              <Plus className="h-4 w-4" />
              Add Custom Provider
            </button>
          )}
        </div>
      </>)}

      {/* Models Tab */}
      {activeTab === 'models' && (<>
        {/* Defaults */}
        <p className="text-xs font-semibold uppercase tracking-[0.2em] text-muted-foreground mb-2">Default Models</p>
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
          {([
            { label: 'Completion', type: 'completion' as ModelCapability, value: defaultModel },
            { label: 'Text to Speech', type: 'tts' as ModelCapability, value: defaultTtsModel },
            { label: 'Speech to Text', type: 'stt' as ModelCapability, value: defaultSttModel },
          ]).map((card) => (
            <button
              key={card.type}
              type="button"
              onClick={() => {
                setSelectorDialogCapability(card.type);
                setSelectorDialogOpen(true);
              }}
              className="rounded-xl border border-border/70 bg-card px-4 py-3 shadow-sm text-left hover:bg-muted/30 transition group"
            >
              <div className="flex items-center gap-2 mb-1">
                <CapabilityBadge type={card.type} />
                <span className="text-[10px] font-medium text-muted-foreground/70 uppercase tracking-wider">{card.label}</span>
              </div>
              <p className="text-sm font-mono text-foreground truncate">
                {card.value || <span className="text-muted-foreground italic font-sans">Not set</span>}
              </p>
            </button>
          ))}
        </div>

        {/* Completion Models */}
        {renderModelTable('completion', 'Completion Models', groupedModels)}

        {/* Voice Models (TTS + STT) */}
        {renderModelTable('voice', 'Voice Models', groupedModels)}

        {/* Add custom provider link */}
        <button
          type="button"
          onClick={() => {
            setShowAddProvider(true);
            onTabChange?.('providers');
          }}
          className="flex items-center gap-2 text-xs text-muted-foreground/60 hover:text-foreground transition"
        >
          <Plus className="h-3.5 w-3.5" />
          Add Custom Provider
        </button>
      </>)}

      {/* Model Selector Dialog */}
      {selectorDialogOpen && (
        <ModelSelectorDialog
          open={selectorDialogOpen}
          onClose={() => setSelectorDialogOpen(false)}
          capability={selectorDialogCapability}
          models={allModels}
          onSelect={async (providerId, modelId) => {
            const fullId = `${providerId}/${modelId}`;
            await handleSetDefault(selectorDialogCapability, fullId);
            setSelectorDialogOpen(false);
          }}
        />
      )}

      {/* Voice Preview Dialog */}
      {voiceDialogOpen && homeClient && (
        <VoicePreviewDialog
          open={voiceDialogOpen}
          onClose={() => setVoiceDialogOpen(false)}
          providers={providers}
          isProviderConfigured={isProviderConfigured}
          homeClient={homeClient}
          onOpenProviders={() => setActiveTab('providers')}
          initialModelId={voiceDialogModelId}
          initialProviderId={voiceDialogProviderId}
        />
      )}
    </div>
  );
}

// ---------- ModelSelectorDialog ----------

interface ModelRow {
  id: string;
  name: string;
  providerId: string;
  providerLabel: string;
  type: ModelCapability;
  isCustom: boolean;
  providerConfigured: boolean;
  contextWindow?: number;
  pricing?: ModelPricing;
  voices?: TtsVoiceInfo[];
}

interface ModelSelectorDialogProps {
  open: boolean;
  onClose: () => void;
  capability: ModelCapability;
  models: ModelRow[];
  onSelect: (providerId: string, modelId: string) => void;
}

const capabilityTitle: Record<ModelCapability, string> = {
  completion: 'Select Default Completion Model',
  tts: 'Select Default Text to Speech Model',
  stt: 'Select Default Speech to Text Model',
};

function ModelSelectorDialog({ open, onClose, capability, models, onSelect }: ModelSelectorDialogProps) {
  if (!open) return null;

  const filtered = models.filter((m) => m.type === capability);

  // Group by provider
  const groups: Array<{ providerId: string; providerLabel: string; models: ModelRow[] }> = [];
  const map = new Map<string, ModelRow[]>();
  const order: string[] = [];
  for (const m of filtered) {
    if (!map.has(m.providerId)) {
      map.set(m.providerId, []);
      order.push(m.providerId);
    }
    map.get(m.providerId)!.push(m);
  }
  for (const pid of order) {
    const ms = map.get(pid)!;
    groups.push({ providerId: pid, providerLabel: ms[0].providerLabel, models: ms });
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={onClose}>
      <div
        className="w-full max-w-2xl max-h-[80vh] rounded-xl border border-border bg-card shadow-xl flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-border/60">
          <h2 className="text-base font-semibold text-foreground">{capabilityTitle[capability]}</h2>
          <button type="button" onClick={onClose} className="text-muted-foreground hover:text-foreground transition">
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Table */}
        <div className="flex-1 overflow-y-auto">
          {/* Table header */}
          <div className="grid grid-cols-[1fr_70px_110px_70px] gap-2 px-6 py-2 border-b border-border/40 bg-muted/30 sticky top-0">
            <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">Model</span>
            <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground text-right">Context</span>
            <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground text-right">Cost / 1M</span>
            <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground text-right">Cached</span>
          </div>

          {groups.length === 0 && (
            <div className="px-6 py-8 text-center">
              <Layers className="h-8 w-8 text-muted-foreground/30 mx-auto mb-3" />
              <p className="text-sm text-muted-foreground">No models available for this type.</p>
            </div>
          )}

          {groups.map((group) => (
            <div key={group.providerId}>
              <div className="flex items-center gap-3 px-6 py-2 bg-muted/20">
                <div className="h-px flex-1 bg-border/50" />
                <span className="text-[10px] font-semibold uppercase tracking-[0.15em] text-muted-foreground/70 shrink-0">
                  {group.providerLabel}
                </span>
                <div className="h-px flex-1 bg-border/50" />
              </div>
              {group.models.map((model) => {
                const configured = model.providerConfigured;
                return (
                  <button
                    key={`${model.providerId}-${model.id}`}
                    type="button"
                    disabled={!configured}
                    onClick={() => configured && onSelect(model.providerId, model.id)}
                    className={`w-full grid grid-cols-[1fr_70px_110px_70px] gap-2 items-center px-6 py-2.5 border-b border-border/20 last:border-b-0 text-left transition ${
                      configured
                        ? 'hover:bg-primary/5 cursor-pointer'
                        : 'opacity-40 cursor-not-allowed'
                    }`}
                  >
                    <div className="flex items-center gap-2 min-w-0">
                      <span className={`text-sm truncate ${model.isCustom ? 'font-mono' : ''} text-foreground`}>
                        {model.name}
                      </span>
                      {model.isCustom && (
                        <span className="text-[10px] text-muted-foreground/50 bg-muted/50 px-1.5 py-0.5 rounded shrink-0">custom</span>
                      )}
                    </div>
                    <div className="text-right">
                      {capability === 'completion' && model.contextWindow ? (
                        <span className="text-xs font-mono text-muted-foreground">
                          {formatContextWindow(model.contextWindow)}
                        </span>
                      ) : (
                        <span className="text-xs text-muted-foreground/30">&mdash;</span>
                      )}
                    </div>
                    <div className="text-right">
                      {model.pricing?.type === 'completion' ? (
                        <span className="text-xs font-mono text-muted-foreground">
                          ${model.pricing.input.toFixed(2)} / ${model.pricing.output.toFixed(2)}
                        </span>
                      ) : model.pricing?.type === 'tts' ? (
                        <span className="text-xs font-mono text-muted-foreground">
                          ${model.pricing.per_1m_chars.toFixed(2)} / 1M chars
                        </span>
                      ) : model.pricing?.type === 'stt' ? (
                        <span className="text-xs font-mono text-muted-foreground">
                          ${model.pricing.per_minute.toFixed(3)} / min
                        </span>
                      ) : (
                        <span className="text-xs text-muted-foreground/30">&mdash;</span>
                      )}
                    </div>
                  </button>
                );
              })}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
