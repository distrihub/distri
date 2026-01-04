import { useCallback, useEffect, useMemo, useState } from 'react';
import { Check, Plus, Trash2 } from 'lucide-react';
import { useDistriHomeClient } from '../DistriHomeProvider';
import type { ProviderSecretDefinition } from '../DistriHomeClient';

// Types
export interface Secret {
  id: string;
  key: string;
  masked_value: string;
  created_at?: string;
  updated_at?: string;
}

// Default definitions used while loading or if API fails
const DEFAULT_PROVIDER_DEFINITIONS: ProviderSecretDefinition[] = [
  {
    id: 'openai',
    label: 'OpenAI',
    keys: [{ key: 'OPENAI_API_KEY', label: 'API key', placeholder: 'sk-...', required: true }],
  },
  {
    id: 'anthropic',
    label: 'Anthropic',
    keys: [{ key: 'ANTHROPIC_API_KEY', label: 'API key', placeholder: 'sk-ant-...', required: true }],
  },
  {
    id: 'gemini',
    label: 'Google Gemini',
    keys: [{ key: 'GEMINI_API_KEY', label: 'API key', placeholder: 'AIza...', required: true }],
  },
];

export interface SecretsViewProps {
  className?: string;
}

export function SecretsView({ className }: SecretsViewProps) {
  const homeClient = useDistriHomeClient();
  const [secrets, setSecrets] = useState<Secret[]>([]);
  const [providerDefinitions, setProviderDefinitions] = useState<ProviderSecretDefinition[]>(DEFAULT_PROVIDER_DEFINITIONS);
  const [selectedProviderId, setSelectedProviderId] = useState<string | null>(null);
  const [providerValues, setProviderValues] = useState<Record<string, string>>({});
  const [customKey, setCustomKey] = useState('');
  const [customValue, setCustomValue] = useState('');
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Filter out the 'custom' provider from definitions for the cards
  const providerCards = useMemo(
    () => providerDefinitions.filter((p) => p.id !== 'custom'),
    [providerDefinitions],
  );

  // Get custom secrets (ones not matching any provider key)
  const allProviderKeys = useMemo(() => {
    const keys = new Set<string>();
    for (const provider of providerDefinitions) {
      for (const keyDef of provider.keys) {
        keys.add(keyDef.key);
      }
    }
    return keys;
  }, [providerDefinitions]);

  const customSecrets = useMemo(
    () => secrets.filter((s) => !allProviderKeys.has(s.key)),
    [secrets, allProviderKeys],
  );

  // Check if a provider has all required keys configured
  const isProviderConfigured = useCallback(
    (provider: ProviderSecretDefinition) => {
      const requiredKeys = provider.keys.filter((k) => k.required);
      if (requiredKeys.length === 0) return false;
      return requiredKeys.every((keyDef) => secrets.some((s) => s.key === keyDef.key));
    },
    [secrets],
  );

  // Get secret for a specific key
  const getSecretForKey = useCallback(
    (key: string) => secrets.find((s) => s.key === key),
    [secrets],
  );

  const load = useCallback(async () => {
    if (!homeClient) return;
    setLoading(true);
    setError(null);
    try {
      const [secretsResponse, definitionsResponse] = await Promise.all([
        homeClient.listSecrets(),
        homeClient.listProviderDefinitions().catch(() => null),
      ]);
      setSecrets(secretsResponse ?? []);
      if (definitionsResponse && definitionsResponse.length > 0) {
        setProviderDefinitions(definitionsResponse);
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to load secrets';
      setError(message);
    } finally {
      setLoading(false);
    }
  }, [homeClient]);

  useEffect(() => {
    void load();
  }, [load]);

  const handleSaveProviderKey = async (key: string) => {
    if (!homeClient) return;
    const value = providerValues[key]?.trim();
    if (!value) return;
    setSaving(key);
    setError(null);
    try {
      await homeClient.createSecret(key, value);
      setProviderValues((prev) => ({ ...prev, [key]: '' }));
      await load();
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to save secret';
      setError(message);
    } finally {
      setSaving(null);
    }
  };

  const handleSaveCustom = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!homeClient) return;
    const key = customKey.trim().toUpperCase();
    const value = customValue.trim();
    if (!key || !value) return;
    setSaving('custom');
    setError(null);
    try {
      await homeClient.createSecret(key, value);
      setCustomKey('');
      setCustomValue('');
      await load();
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to save secret';
      setError(message);
    } finally {
      setSaving(null);
    }
  };

  const handleDelete = async (id: string) => {
    if (!homeClient) return;
    setError(null);
    try {
      await homeClient.deleteSecret(id);
      await load();
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to delete secret';
      setError(message);
    }
  };

  const selectedProvider = providerCards.find((p) => p.id === selectedProviderId);

  return (
    <div className={`flex-1 overflow-y-auto ${className ?? ''}`}>
      <div className="mx-auto w-full max-w-5xl px-6 py-8 lg:px-10">
        {/* Provider Cards */}
        <div className="rounded-2xl border border-border/70 bg-card p-6 shadow-sm">
          <div>
            <p className="text-xs font-semibold uppercase tracking-[0.3em] text-muted-foreground">
              Providers
            </p>
            <h3 className="mt-2 text-lg font-semibold text-foreground">API Keys</h3>
            <p className="mt-1 text-sm text-muted-foreground">
              Configure API keys for LLM providers.
            </p>
          </div>

          {loading ? (
            <div className="mt-6 text-sm text-muted-foreground">Loading…</div>
          ) : (
            <div className="mt-6 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {providerCards.map((provider) => {
                const configured = isProviderConfigured(provider);
                const isSelected = selectedProviderId === provider.id;
                return (
                  <button
                    key={provider.id}
                    type="button"
                    onClick={() => setSelectedProviderId(isSelected ? null : provider.id)}
                    className={`relative flex items-center gap-3 rounded-xl border p-4 text-left transition ${
                      isSelected
                        ? 'border-primary bg-primary/5'
                        : configured
                          ? 'border-emerald-500/50 bg-emerald-500/5 hover:border-emerald-500'
                          : 'border-border/70 hover:border-primary/50'
                    }`}
                  >
                    <div className="flex-1">
                      <div className="text-sm font-semibold text-foreground">{provider.label}</div>
                      <div className="mt-0.5 text-xs text-muted-foreground">
                        {configured ? 'Configured' : 'Not configured'}
                      </div>
                    </div>
                    {configured && (
                      <div className="flex h-6 w-6 items-center justify-center rounded-full bg-emerald-500/20 text-emerald-500">
                        <Check className="h-3.5 w-3.5" />
                      </div>
                    )}
                  </button>
                );
              })}
            </div>
          )}

          {/* Selected Provider Form */}
          {selectedProvider && (
            <div className="mt-6 rounded-xl border border-border/60 bg-muted/20 p-4">
              <div className="mb-4 flex items-center justify-between">
                <h4 className="text-sm font-semibold text-foreground">{selectedProvider.label}</h4>
              </div>
              <div className="space-y-3">
                {selectedProvider.keys.map((keyDef) => {
                  const existingSecret = getSecretForKey(keyDef.key);
                  return (
                    <div key={keyDef.key} className="flex items-center gap-3">
                      <div className="w-32 shrink-0">
                        <span className="text-xs font-medium text-muted-foreground">{keyDef.label}</span>
                      </div>
                      {existingSecret ? (
                        <>
                          <div className="flex-1 rounded-md border border-border/70 bg-background px-3 py-2 text-sm font-mono text-muted-foreground">
                            {existingSecret.masked_value}
                          </div>
                          <button
                            type="button"
                            onClick={() => handleDelete(existingSecret.id)}
                            className="text-muted-foreground transition hover:text-destructive"
                            title="Delete"
                          >
                            <Trash2 className="h-4 w-4" />
                          </button>
                        </>
                      ) : (
                        <>
                          <input
                            type="password"
                            value={providerValues[keyDef.key] || ''}
                            onChange={(e) =>
                              setProviderValues((prev) => ({ ...prev, [keyDef.key]: e.target.value }))
                            }
                            placeholder={keyDef.placeholder}
                            className="flex-1 rounded-md border border-border/70 bg-background px-3 py-2 text-sm text-foreground focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                          />
                          <button
                            type="button"
                            onClick={() => handleSaveProviderKey(keyDef.key)}
                            disabled={saving === keyDef.key || !providerValues[keyDef.key]?.trim()}
                            className="inline-flex h-9 items-center justify-center rounded-md bg-primary px-3 text-sm font-semibold text-primary-foreground transition hover:bg-primary/90 disabled:opacity-50"
                          >
                            {saving === keyDef.key ? 'Saving…' : 'Save'}
                          </button>
                        </>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}
        </div>

        {/* Custom Secrets Section */}
        <div className="mt-6 rounded-2xl border border-border/70 bg-card p-6 shadow-sm">
          <div>
            <p className="text-xs font-semibold uppercase tracking-[0.3em] text-muted-foreground">
              Custom
            </p>
            <h3 className="mt-2 text-lg font-semibold text-foreground">Custom Secrets</h3>
            <p className="mt-1 text-sm text-muted-foreground">
              Add custom environment variables and secrets.
            </p>
          </div>

          {/* Add Custom Secret Form */}
          <form onSubmit={handleSaveCustom} className="mt-6 flex items-end gap-3">
            <div className="w-48">
              <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                Key
              </label>
              <input
                value={customKey}
                onChange={(e) => setCustomKey(e.target.value.toUpperCase())}
                placeholder="MY_SECRET_KEY"
                className="mt-2 h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm font-mono text-foreground focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
              />
            </div>
            <div className="flex-1">
              <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                Value
              </label>
              <input
                type="password"
                value={customValue}
                onChange={(e) => setCustomValue(e.target.value)}
                placeholder="Secret value..."
                className="mt-2 h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
              />
            </div>
            <button
              type="submit"
              disabled={saving === 'custom' || !customKey.trim() || !customValue.trim()}
              className="inline-flex h-10 items-center gap-2 rounded-md bg-primary px-4 text-sm font-semibold text-primary-foreground transition hover:bg-primary/90 disabled:opacity-50"
            >
              <Plus className="h-4 w-4" />
              {saving === 'custom' ? 'Adding…' : 'Add'}
            </button>
          </form>

          {/* Custom Secrets List */}
          {customSecrets.length > 0 && (
            <div className="mt-6 overflow-hidden rounded-xl border border-border/60">
              <div className="grid grid-cols-[1fr_1fr_auto] gap-3 border-b border-border/60 bg-muted/30 px-4 py-3 text-xs font-semibold uppercase tracking-[0.3em] text-muted-foreground">
                <span>Key</span>
                <span>Value</span>
                <span className="text-right">Actions</span>
              </div>
              {customSecrets.map((secret) => (
                <div
                  key={secret.id}
                  className="grid grid-cols-[1fr_1fr_auto] items-center gap-3 border-b border-border/60 px-4 py-3 last:border-b-0"
                >
                  <span className="text-sm font-mono text-foreground">{secret.key}</span>
                  <span className="text-sm font-mono text-muted-foreground">{secret.masked_value}</span>
                  <button
                    type="button"
                    onClick={() => handleDelete(secret.id)}
                    className="inline-flex justify-end text-muted-foreground transition hover:text-destructive"
                  >
                    <Trash2 className="h-4 w-4" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        {error && (
          <div className="mt-6 rounded-xl border border-red-400/50 bg-red-500/10 px-4 py-3 text-sm text-red-700 dark:text-red-200">
            {error}
          </div>
        )}
      </div>
    </div>
  );
}
