import { useCallback, useEffect, useMemo, useState } from 'react';
import { useModels } from '@distri/react';
import { Check, ChevronDown, Loader2, MessageSquare, Plus, Star, Trash2, Volume2, X } from 'lucide-react';
import { useDistriHomeClient } from '../DistriHomeProvider';
import type { Secret, ProviderSecretDefinition, CustomModelEntry, CustomProviderConfig, TtsProviderDefinition } from '../DistriHomeClient';

type AgentSettingsTab = 'completion' | 'tts';

export interface AgentSettingsViewProps {
  className?: string;
}

export function AgentSettingsView({ className }: AgentSettingsViewProps) {
  const homeClient = useDistriHomeClient();
  const { providers: modelProviders } = useModels();

  const [secrets, setSecrets] = useState<Secret[]>([]);
  const [providerDefs, setProviderDefs] = useState<ProviderSecretDefinition[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [defaultModel, setDefaultModel] = useState<string>('');
  const [customModels, setCustomModels] = useState<CustomModelEntry[]>([]);
  const [saving, setSaving] = useState(false);
  const [saveSuccess, setSaveSuccess] = useState(false);

  const [fieldValues, setFieldValues] = useState<Record<string, string>>({});
  const [savingField, setSavingField] = useState<string | null>(null);
  const [newModelInputs, setNewModelInputs] = useState<Record<string, string>>({});
  const [expandedProviders, setExpandedProviders] = useState<Set<string>>(new Set());

  // Custom provider management
  const [showAddProvider, setShowAddProvider] = useState(false);
  const [newProviderName, setNewProviderName] = useState('');
  const [newProviderUrl, setNewProviderUrl] = useState('');
  const [newProviderKey, setNewProviderKey] = useState('');
  const [newProviderProjectId, setNewProviderProjectId] = useState('');

  // Custom providers stored in workspace settings
  const [customProviders, setCustomProviders] = useState<CustomProviderConfig[]>([]);

  // Tab state
  const [activeTab, setActiveTab] = useState<AgentSettingsTab>('completion');

  // TTS state
  const [ttsProviderDefs, setTtsProviderDefs] = useState<TtsProviderDefinition[]>([]);
  const [defaultTtsModel, setDefaultTtsModel] = useState<string>('');
  const [expandedTtsProviders, setExpandedTtsProviders] = useState<Set<string>>(new Set());

  const loadData = useCallback(async () => {
    if (!homeClient) return;
    setLoading(true);
    setError(null);
    try {
      const [settings, secs, defs, ttsDefs] = await Promise.all([
        homeClient.getWorkspaceSettings(),
        homeClient.listSecrets(),
        homeClient.listProviderDefinitions(),
        homeClient.listTtsProviders().catch(() => []),
      ]);
      setSecrets(secs);
      setProviderDefs(defs);
      setDefaultModel(settings?.default_model || '');
      setCustomModels(settings?.custom_models || []);
      setCustomProviders(settings?.custom_providers || []);
      setTtsProviderDefs(ttsDefs);
      setDefaultTtsModel(settings?.default_tts_model || '');
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load settings');
    } finally {
      setLoading(false);
    }
  }, [homeClient]);

  useEffect(() => { loadData(); }, [loadData]);

  const builtinProviders = useMemo(
    () => providerDefs.filter((p) => p.id !== 'custom' && p.id !== 'openai_compat'),
    [providerDefs],
  );

  const getSecret = useCallback(
    (key: string) => secrets.find((s) => s.key === key),
    [secrets],
  );

  const isProviderConfigured = useCallback(
    (provider: ProviderSecretDefinition) => {
      const required = provider.keys.filter((k) => k.required !== false);
      if (required.length === 0) return false;
      return required.every((k) => {
        const s = getSecret(k.key);
        return s && s.masked_value && s.masked_value !== '';
      });
    },
    [getSecret],
  );

  const getModelsForProvider = useCallback(
    (providerId: string) => {
      const mp = modelProviders.find((p) => p.provider_id === providerId);
      return mp?.models ?? [];
    },
    [modelProviders],
  );

  const getCustomModelsForProvider = useCallback(
    (providerId: string) => customModels.filter((m) => m.provider === providerId),
    [customModels],
  );

  // Save all unsaved fields for a provider at once via POST /providers
  const handleSaveProvider = async (providerId: string, keys: ProviderSecretDefinition['keys']) => {
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

  const handleSetDefaultModel = async (model: string) => {
    if (!homeClient) return;
    const newDefault = defaultModel === model ? '' : model;
    setDefaultModel(newDefault);
    setSaving(true);
    try {
      await homeClient.upsertProvider({
        provider_id: '__settings__',
        default_model: newDefault,
      });
      setSaveSuccess(true);
      setTimeout(() => setSaveSuccess(false), 2000);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save');
    } finally {
      setSaving(false);
    }
  };

  const handleAddModel = async (providerId: string) => {
    if (!homeClient) return;
    const modelName = newModelInputs[providerId]?.trim();
    if (!modelName) return;

    const wellKnown = getModelsForProvider(providerId);
    if (wellKnown.some((m) => m.id === modelName)) return;
    if (customModels.some((m) => m.provider === providerId && m.model === modelName)) return;

    const updated = [...customModels, { provider: providerId, model: modelName }];
    setCustomModels(updated);
    setNewModelInputs((prev) => ({ ...prev, [providerId]: '' }));
    try {
      await homeClient.upsertProvider({
        provider_id: '__settings__',
        custom_models: updated,
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save');
    }
  };

  const handleRemoveCustomModel = async (providerId: string, modelName: string) => {
    if (!homeClient) return;
    const fullId = `${providerId}/${modelName}`;
    const updated = customModels.filter((m) => !(m.provider === providerId && m.model === modelName));
    setCustomModels(updated);

    let newDefault = defaultModel;
    if (defaultModel === fullId) {
      newDefault = '';
      setDefaultModel('');
    }
    try {
      await homeClient.upsertProvider({
        provider_id: '__settings__',
        custom_models: updated,
        default_model: newDefault,
      });
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

  const providerMeta = (id: string) => {
    switch (id) {
      case 'openai': return { name: 'OpenAI', desc: 'Use your OpenAI key for GPT models at cost.' };
      case 'anthropic': return { name: 'Anthropic', desc: 'Use your Anthropic key for Claude models at cost.' };
      case 'azure_openai': return { name: 'Azure OpenAI', desc: 'Use OpenAI models through your Azure account.' };
      case 'gemini': return { name: 'Google Gemini', desc: 'Use your Google AI Studio key for Gemini models.' };
      case 'openai_compat': return { name: 'Custom Provider', desc: 'OpenAI-compatible endpoint (vLLM, LiteLLM, LangDB, Ollama, etc).' };
      default: {
        const cp = customProviders.find((p) => p.id === id);
        return cp ? { name: cp.name, desc: cp.base_url } : { name: id, desc: '' };
      }
    }
  };

  // Compute first available model
  const firstAvailable = useMemo(() => {
    if (defaultModel) return null;
    for (const p of builtinProviders) {
      if (!isProviderConfigured(p)) continue;
      const models = getModelsForProvider(p.id);
      if (models.length > 0) return `${p.id}/${models[0].id}`;
      const custom = getCustomModelsForProvider(p.id);
      if (custom.length > 0) return `${p.id}/${custom[0].model}`;
    }
    for (const cp of customProviders) {
      const custom = getCustomModelsForProvider(cp.id);
      if (custom.length > 0) return `${cp.id}/${custom[0].model}`;
    }
    return null;
  }, [defaultModel, builtinProviders, customProviders, isProviderConfigured, getModelsForProvider, getCustomModelsForProvider]);

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
      </div>
    );
  }

  const toggleProvider = (id: string) => {
    setExpandedProviders((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const toggleTtsProvider = (id: string) => {
    setExpandedTtsProviders((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const isTtsProviderConfigured = (provider: TtsProviderDefinition) => {
    const required = provider.keys.filter((k) => k.required);
    if (required.length === 0) return false;
    return required.every((k) => {
      const s = getSecret(k.key);
      return s && s.masked_value && s.masked_value !== '';
    });
  };

  const handleSetDefaultTtsModel = async (model: string) => {
    if (!homeClient) return;
    const newDefault = defaultTtsModel === model ? '' : model;
    setDefaultTtsModel(newDefault);
    setSaving(true);
    try {
      await homeClient.updateWorkspaceSettings({ default_tts_model: newDefault || null });
      setSaveSuccess(true);
      setTimeout(() => setSaveSuccess(false), 2000);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save');
    } finally {
      setSaving(false);
    }
  };

  const handleSaveTtsProvider = async (providerId: string, keys: TtsProviderDefinition['keys']) => {
    if (!homeClient) return;
    const toSave = keys.filter((k) => {
      const val = fieldValues[k.key]?.trim();
      return val && !getSecret(k.key);
    });
    if (toSave.length === 0) return;
    setSavingField('__tts_provider__');
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

  // Render a provider section (reused for builtin and custom)
  const renderProviderSection = (
    providerId: string,
    providerName: string,
    providerDesc: string,
    configured: boolean,
    keys: ProviderSecretDefinition['keys'],
    isCustomProvider: boolean,
  ) => {
    const isExpanded = expandedProviders.has(providerId);
    const wellKnownModels = getModelsForProvider(providerId);
    const userModels = getCustomModelsForProvider(providerId);

    const allModels: Array<{ id: string; name: string; isCustom: boolean }> = [
      ...wellKnownModels.map((m) => ({ id: m.id, name: m.name, isCustom: false })),
      ...userModels
        .filter((cm) => !wellKnownModels.some((wk) => wk.id === cm.model))
        .map((cm) => ({ id: cm.model, name: cm.model, isCustom: true })),
    ];

    return (
      <div key={providerId} className="rounded-xl border border-border/70 bg-card shadow-sm overflow-hidden">
        {/* Header */}
        <button
          type="button"
          onClick={() => toggleProvider(providerId)}
          className="flex w-full items-center justify-between px-6 py-4 text-left hover:bg-muted/30 transition"
        >
          <div>
            <h3 className="text-sm font-semibold text-foreground">{providerName}</h3>
            {!isExpanded ? (
              <p className="text-xs text-muted-foreground mt-0.5">
                {configured ? 'Configured' : 'Not configured'}
              </p>
            ) : providerDesc && (
              <p className="text-xs text-muted-foreground mt-0.5">{providerDesc}</p>
            )}
          </div>
          <div className="flex items-center gap-2">
            {isCustomProvider && (
              <span
                role="button"
                onClick={(e) => { e.stopPropagation(); handleRemoveCustomProvider(providerId); }}
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

        {/* Accordion body */}
        {isExpanded && <>
        {/* Config fields */}
        {keys.length > 0 && (() => {
          const hasUnsaved = keys.some((k) => !getSecret(k.key) && fieldValues[k.key]?.trim());
          return (
          <div className="px-6 py-4 space-y-3">
            {keys.map((keyDef) => {
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
                      {isSensitive ? '••••••••' : existing.masked_value}
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
                  onClick={() => handleSaveProvider(providerId, keys)}
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

        {/* Models section */}
        <div className={`border-t border-border/40 px-6 py-4 ${!configured ? 'opacity-50' : ''}`}>
          <p className="text-xs font-medium text-muted-foreground mb-2">Models</p>
          <div className="space-y-0.5">
            {allModels.map((model) => {
              const fullId = `${providerId}/${model.id}`;
              const isDefault = defaultModel === fullId;

              return (
                <div
                  key={model.id}
                  className="group flex items-center justify-between rounded-lg px-3 py-2 hover:bg-muted/50 transition"
                >
                  <div className="flex items-center gap-2">
                    <span className={`text-sm ${model.isCustom ? 'font-mono' : ''} text-foreground`}>
                      {model.name}
                    </span>
                    {model.isCustom && (
                      <span className="text-[10px] text-muted-foreground/50 bg-muted/50 px-1.5 py-0.5 rounded">custom</span>
                    )}
                  </div>
                  <div className="flex items-center gap-1">
                    {model.isCustom && (
                      <button
                        type="button"
                        onClick={() => handleRemoveCustomModel(providerId, model.id)}
                        className="opacity-0 group-hover:opacity-100 text-muted-foreground/40 hover:text-destructive transition p-1"
                        title="Remove model"
                      >
                        <Trash2 className="h-3 w-3" />
                      </button>
                    )}
                    <button
                      type="button"
                      onClick={() => configured && handleSetDefaultModel(fullId)}
                      disabled={saving || !configured}
                      className={`flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium transition ${
                        isDefault
                          ? 'bg-primary/10 text-primary'
                          : 'opacity-0 group-hover:opacity-100 text-muted-foreground/60 hover:text-primary hover:bg-primary/5'
                      }`}
                    >
                      <Star className={`h-3 w-3 ${isDefault ? 'fill-primary' : ''}`} />
                      {isDefault && 'Default'}
                    </button>
                  </div>
                </div>
              );
            })}

            {allModels.length === 0 && (
              <p className="text-xs text-muted-foreground/50 px-3 py-1">No models configured. Add one below.</p>
            )}

            {/* Add model */}
            <div className="flex items-center gap-2 pt-1">
              <button
                type="button"
                onClick={() => handleAddModel(providerId)}
                disabled={!newModelInputs[providerId]?.trim() || saving}
                className="flex h-7 w-7 items-center justify-center rounded-md border border-border/40 text-muted-foreground/40 transition hover:border-primary/60 hover:text-primary disabled:opacity-50 shrink-0"
                title="Add model"
              >
                <Plus className="h-3.5 w-3.5" />
              </button>
              <input
                type="text"
                value={newModelInputs[providerId] || ''}
                onChange={(e) =>
                  setNewModelInputs((prev) => ({ ...prev, [providerId]: e.target.value }))
                }
                onKeyDown={(e) => {
                  if (e.key === 'Enter') handleAddModel(providerId);
                }}
                placeholder="Add model name..."
                className="flex-1 rounded-lg border border-border/40 bg-transparent px-2 py-1.5 text-xs text-foreground placeholder:text-muted-foreground/30 focus:border-primary focus:outline-none"
              />
            </div>
          </div>
        </div>
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

      {/* ─── Tab Bar ─── */}
      <div className="flex gap-1 rounded-lg bg-muted/50 p-1">
        <button
          type="button"
          onClick={() => setActiveTab('completion')}
          className={`flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium transition ${
            activeTab === 'completion'
              ? 'bg-background text-foreground shadow-sm'
              : 'text-muted-foreground hover:text-foreground'
          }`}
        >
          <MessageSquare className="h-3.5 w-3.5" />
          Completion
        </button>
        <button
          type="button"
          onClick={() => setActiveTab('tts')}
          className={`flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium transition ${
            activeTab === 'tts'
              ? 'bg-background text-foreground shadow-sm'
              : 'text-muted-foreground hover:text-foreground'
          }`}
        >
          <Volume2 className="h-3.5 w-3.5" />
          Text-to-Speech
        </button>
      </div>

      {/* ─── Completion Tab ─── */}
      {activeTab === 'completion' && (<>
        {/* Default model banner */}
        <div className="rounded-xl border border-border/70 bg-card px-6 py-4 shadow-sm">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-xs font-semibold uppercase tracking-[0.2em] text-muted-foreground">Default Model</p>
              {defaultModel ? (
                <p className="mt-1 text-sm font-mono text-foreground">{defaultModel}</p>
              ) : (
                <p className="mt-1 text-sm text-muted-foreground">
                  Not set{firstAvailable && <> — first available model will be used: <span className="font-mono text-foreground">{firstAvailable}</span></>}
                </p>
              )}
              <p className="mt-1 text-xs text-muted-foreground/70">Choose the default model below under the list of available providers.</p>
            </div>
            {defaultModel && <Star className="h-4 w-4 fill-primary text-primary shrink-0" />}
          </div>
        </div>

        {/* Built-in providers */}
        {builtinProviders.map((provider) => {
          const configured = isProviderConfigured(provider);
          const { name, desc } = providerMeta(provider.id);
          return renderProviderSection(provider.id, name, desc, configured, provider.keys, false);
        })}

        {/* Custom providers */}
        {customProviders.map((cp) => {
          const keyPrefix = cp.id.toUpperCase();
          const hasKey = !!getSecret(`${keyPrefix}_API_KEY`);
          const keys = [
            { key: `${keyPrefix}_BASE_URL`, label: 'API URL', placeholder: cp.base_url || 'https://api.example.com/v1', required: true, sensitive: false },
            { key: `${keyPrefix}_API_KEY`, label: 'API key', placeholder: 'sk-...', required: true, sensitive: true },
            ...(cp.project_id ? [{ key: `${keyPrefix}_PROJECT_ID`, label: 'Project ID', placeholder: cp.project_id, required: false, sensitive: false }] : []),
          ];
          return renderProviderSection(cp.id, cp.name, cp.base_url, hasKey, keys, true);
        })}

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

      {/* ─── TTS Tab ─── */}
      {activeTab === 'tts' && (<>
        {/* Default TTS model banner */}
        <div className="rounded-xl border border-border/70 bg-card px-6 py-4 shadow-sm">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-xs font-semibold uppercase tracking-[0.2em] text-muted-foreground">Default TTS Model</p>
              {defaultTtsModel ? (
                <p className="mt-1 text-sm font-mono text-foreground">{defaultTtsModel}</p>
              ) : (
                <p className="mt-1 text-sm text-muted-foreground">Not set — configure a TTS provider below.</p>
              )}
              <p className="mt-1 text-xs text-muted-foreground/70">Choose the default TTS model below under the list of available providers.</p>
            </div>
            {defaultTtsModel && <Star className="h-4 w-4 fill-primary text-primary shrink-0" />}
          </div>
        </div>

        {/* TTS providers */}
        {ttsProviderDefs.map((provider) => {
          const configured = isTtsProviderConfigured(provider);
          const isExpanded = expandedTtsProviders.has(provider.id);

          return (
            <div key={`tts-${provider.id}`} className="rounded-xl border border-border/70 bg-card shadow-sm overflow-hidden">
              <button
                type="button"
                onClick={() => toggleTtsProvider(provider.id)}
                className="flex w-full items-center justify-between px-6 py-4 text-left hover:bg-muted/30 transition"
              >
                <div>
                  <h3 className="text-sm font-semibold text-foreground">{provider.label}</h3>
                  {!isExpanded ? (
                    <p className="text-xs text-muted-foreground mt-0.5">
                      {configured ? 'Configured' : 'Not configured'}
                      {' · '}{provider.models.length} model{provider.models.length !== 1 ? 's' : ''}
                    </p>
                  ) : (
                    <p className="text-xs text-muted-foreground mt-0.5">
                      {provider.models.length} model{provider.models.length !== 1 ? 's' : ''} available
                    </p>
                  )}
                </div>
                <div className="flex items-center gap-2">
                  {configured && (
                    <div className="flex h-6 w-6 items-center justify-center rounded-full bg-emerald-500/20 text-emerald-500 shrink-0">
                      <Check className="h-3.5 w-3.5" />
                    </div>
                  )}
                  <ChevronDown className={`h-4 w-4 text-muted-foreground transition-transform ${isExpanded ? 'rotate-180' : ''}`} />
                </div>
              </button>

              {isExpanded && (<>
                {/* API key fields */}
                {provider.keys.length > 0 && (() => {
                  const hasUnsaved = provider.keys.some((k) => !getSecret(k.key) && fieldValues[k.key]?.trim());
                  return (
                    <div className="px-6 py-4 space-y-3">
                      {provider.keys.map((keyDef) => {
                        const existing = getSecret(keyDef.key);
                        return (
                          <div key={keyDef.key}>
                            <div className="flex items-center justify-between mb-1.5">
                              <label className="text-xs font-medium text-muted-foreground">
                                {keyDef.label}
                                {!keyDef.required && <span className="ml-1 text-muted-foreground/40">(optional)</span>}
                              </label>
                              {existing && (
                                <button type="button" onClick={() => handleDeleteField(existing.key)} className="text-muted-foreground/40 transition hover:text-destructive" title="Remove">
                                  <Trash2 className="h-3.5 w-3.5" />
                                </button>
                              )}
                            </div>
                            {existing ? (
                              <div className="rounded-lg border border-border/50 bg-muted/30 px-3 py-2 text-sm font-mono text-foreground">
                                {keyDef.sensitive ? '••••••••' : existing.masked_value}
                              </div>
                            ) : (
                              <input
                                type={keyDef.sensitive ? 'password' : 'text'}
                                value={fieldValues[keyDef.key] || ''}
                                onChange={(e) => setFieldValues((prev) => ({ ...prev, [keyDef.key]: e.target.value }))}
                                placeholder={keyDef.placeholder}
                                className="w-full rounded-lg border border-border/70 bg-background px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground/40 focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                              />
                            )}
                          </div>
                        );
                      })}
                      {hasUnsaved && (
                        <div className="flex justify-end pt-1">
                          <button type="button" onClick={() => handleSaveTtsProvider(provider.id, provider.keys)} disabled={savingField === '__tts_provider__'} className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition hover:bg-primary/90 disabled:opacity-50">
                            {savingField === '__tts_provider__' ? 'Saving...' : 'Save'}
                          </button>
                        </div>
                      )}
                    </div>
                  );
                })()}

                {/* Models & voices */}
                <div className={`border-t border-border/40 px-6 py-4 ${!configured ? 'opacity-50' : ''}`}>
                  <p className="text-xs font-medium text-muted-foreground mb-2">Models</p>
                  <div className="space-y-0.5">
                    {provider.models.map((model) => {
                      const fullId = `${provider.id}/${model.id}`;
                      const isDefault = defaultTtsModel === fullId;
                      return (
                        <div key={model.id}>
                          <div className="group flex items-center justify-between rounded-lg px-3 py-2 hover:bg-muted/50 transition">
                            <div>
                              <span className="text-sm text-foreground">{model.name}</span>
                              <span className="ml-2 text-[10px] text-muted-foreground/50">
                                {model.voices.length} voice{model.voices.length !== 1 ? 's' : ''}
                              </span>
                            </div>
                            <button
                              type="button"
                              onClick={() => configured && handleSetDefaultTtsModel(fullId)}
                              disabled={saving || !configured}
                              className={`flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium transition ${
                                isDefault
                                  ? 'bg-primary/10 text-primary'
                                  : 'opacity-0 group-hover:opacity-100 text-muted-foreground/60 hover:text-primary hover:bg-primary/5'
                              }`}
                            >
                              <Star className={`h-3 w-3 ${isDefault ? 'fill-primary' : ''}`} />
                              {isDefault && 'Default'}
                            </button>
                          </div>
                          {/* Voice list under model */}
                          <div className="ml-6 space-y-0">
                            {model.voices.map((v) => (
                              <div key={v.id} className="flex items-center gap-2 px-3 py-1 text-xs text-muted-foreground">
                                <span className="text-foreground">{v.name}</span>
                                {v.description && <span className="text-muted-foreground/50">— {v.description}</span>}
                              </div>
                            ))}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </div>
              </>)}
            </div>
          );
        })}

        {ttsProviderDefs.length === 0 && (
          <div className="rounded-xl border border-dashed border-border/70 bg-card/50 px-6 py-8 text-center">
            <Volume2 className="h-8 w-8 text-muted-foreground/30 mx-auto mb-3" />
            <p className="text-sm text-muted-foreground">No TTS providers available.</p>
          </div>
        )}
      </>)}
    </div>
  );
}
