import { useCallback, useEffect, useMemo, useState } from 'react';
import { Trash2 } from 'lucide-react';
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
  {
    id: 'custom',
    label: 'Custom',
    keys: [],
  },
];

export interface SecretsViewProps {
  className?: string;
}

export function SecretsView({ className }: SecretsViewProps) {
  const homeClient = useDistriHomeClient();
  const [secrets, setSecrets] = useState<Secret[]>([]);
  const [providerDefinitions, setProviderDefinitions] = useState<ProviderSecretDefinition[]>(DEFAULT_PROVIDER_DEFINITIONS);
  const [providerId, setProviderId] = useState(DEFAULT_PROVIDER_DEFINITIONS[0]?.id ?? '');
  const [providerKey, setProviderKey] = useState(DEFAULT_PROVIDER_DEFINITIONS[0]?.keys[0]?.key ?? '');
  const [secretKey, setSecretKey] = useState('');
  const [secretValue, setSecretValue] = useState('');
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const providerDefinition = useMemo(
    () => providerDefinitions.find((provider) => provider.id === providerId),
    [providerId, providerDefinitions],
  );

  const providerKeyOptions = providerDefinition?.keys ?? [];

  useEffect(() => {
    if (providerId === 'custom') {
      setProviderKey('');
      return;
    }
    if (providerKeyOptions.length > 0 && !providerKeyOptions.some((item) => item.key === providerKey)) {
      setProviderKey(providerKeyOptions[0]?.key ?? '');
    }
  }, [providerKey, providerKeyOptions, providerId]);

  const load = useCallback(async () => {
    if (!homeClient) return;
    setLoading(true);
    setError(null);
    try {
      // Load both secrets and provider definitions in parallel
      const [secretsResponse, definitionsResponse] = await Promise.all([
        homeClient.listSecrets(),
        homeClient.listProviderDefinitions().catch(() => null), // Gracefully fall back to defaults
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

  const handleSave = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!homeClient) return;
    const key =
      providerId === 'custom' ? secretKey.trim().toUpperCase() : providerKey.trim().toUpperCase();
    if (!key || !secretValue.trim()) return;
    setSaving(true);
    setError(null);
    try {
      await homeClient.createSecret(key, secretValue.trim());
      setSecretKey('');
      setSecretValue('');
      await load();
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to save secret';
      setError(message);
    } finally {
      setSaving(false);
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

  const providerLabelByKey = useCallback((key: string) => {
    return (
      providerDefinitions.find((provider) =>
        provider.keys.some((definition) => definition.key === key),
      )?.label ?? 'Custom'
    );
  }, [providerDefinitions]);

  const providerKeyLabel = useCallback((key: string) => {
    for (const provider of providerDefinitions) {
      const match = provider.keys.find((definition) => definition.key === key);
      if (match) return match.label;
    }
    return key;
  }, [providerDefinitions]);

  return (
    <div className={`flex-1 overflow-y-auto ${className ?? ''}`}>
      <div className="mx-auto w-full max-w-5xl px-6 py-8 lg:px-10">
        <div className="rounded-2xl border border-border/70 bg-card p-6 shadow-sm">
          <div>
            <p className="text-xs font-semibold uppercase tracking-[0.3em] text-muted-foreground">
              Secrets
            </p>
            <h3 className="mt-2 text-lg font-semibold text-foreground">Secrets</h3>
            <p className="mt-1 text-sm text-muted-foreground">
              Manage provider API keys and custom secret values.
            </p>
          </div>
          <form onSubmit={handleSave} className="mt-6 grid gap-4 lg:grid-cols-[180px_200px_1fr_auto]">
            <div>
              <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                Provider
              </label>
              <select
                value={providerId}
                onChange={(event) => setProviderId(event.target.value)}
                className="mt-2 h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground shadow-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
              >
                {providerDefinitions.map((provider) => (
                  <option key={provider.id} value={provider.id}>
                    {provider.label}
                  </option>
                ))}
              </select>
            </div>
            <div>
              <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                Key
              </label>
              {providerId === 'custom' ? (
                <input
                  value={secretKey}
                  onChange={(event) => setSecretKey(event.target.value.toUpperCase())}
                  placeholder="MY_SECRET"
                  className="mt-2 h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm font-mono text-foreground shadow-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                />
              ) : providerKeyOptions.length === 1 ? (
                <div className="mt-2 h-10 flex items-center px-3 text-sm text-foreground font-mono bg-muted/30 rounded-md border border-border/70">
                  {providerKeyOptions[0].label}
                </div>
              ) : (
                <select
                  value={providerKey}
                  onChange={(event) => setProviderKey(event.target.value)}
                  className="mt-2 h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground shadow-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                >
                  {providerKeyOptions.map((option) => (
                    <option key={option.key} value={option.key}>
                      {option.label}
                    </option>
                  ))}
                </select>
              )}
            </div>
            <div>
              <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                Value
              </label>
              <input
                value={secretValue}
                onChange={(event) => setSecretValue(event.target.value)}
                placeholder={
                  providerId === 'custom'
                    ? 'Value'
                    : providerKeyOptions.find((item) => item.key === providerKey)?.placeholder ?? 'sk-...'
                }
                type="password"
                className="mt-2 h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground shadow-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
              />
            </div>
            <button
              type="submit"
              disabled={saving || !secretValue.trim() || (providerId === 'custom' && !secretKey.trim())}
              className="inline-flex h-10 items-center justify-center rounded-md bg-primary px-4 text-sm font-semibold text-primary-foreground shadow-sm shadow-primary/20 transition hover:bg-primary/90 disabled:opacity-50"
            >
              {saving ? 'Saving…' : 'Save'}
            </button>
          </form>

          <div className="mt-6 overflow-hidden rounded-xl border border-border/60">
            <div className="grid grid-cols-[160px_1fr_1fr_auto] gap-3 border-b border-border/60 bg-muted/30 px-4 py-3 text-xs font-semibold uppercase tracking-[0.3em] text-muted-foreground">
              <span>Provider</span>
              <span>Key</span>
              <span>Value</span>
              <span className="text-right">Actions</span>
            </div>
            {loading ? (
              <div className="px-4 py-3 text-sm text-muted-foreground">Loading…</div>
            ) : secrets.length === 0 ? (
              <div className="px-4 py-3 text-sm text-muted-foreground">No secrets yet.</div>
            ) : (
              secrets.map((secret) => (
                <div
                  key={secret.id}
                  className="grid grid-cols-[160px_1fr_1fr_auto] items-center gap-3 border-b border-border/60 px-4 py-3 last:border-b-0"
                >
                  <span className="text-sm font-semibold text-foreground">
                    {providerLabelByKey(secret.key)}
                  </span>
                  <span className="text-sm font-mono text-foreground">{providerKeyLabel(secret.key)}</span>
                  <span className="text-sm font-mono text-muted-foreground">{secret.masked_value}</span>
                  <button
                    type="button"
                    onClick={() => handleDelete(secret.id)}
                    className="inline-flex justify-end text-muted-foreground transition hover:text-destructive"
                  >
                    <Trash2 className="h-4 w-4" />
                  </button>
                </div>
              ))
            )}
          </div>
        </div>

        {error ? (
          <div className="mt-6 rounded-xl border border-red-400/50 bg-red-500/10 px-4 py-3 text-sm text-red-700 dark:text-red-200">
            {error}
          </div>
        ) : null}
      </div>
    </div>
  );
}
