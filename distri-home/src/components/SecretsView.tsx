import { useCallback, useEffect, useMemo, useState } from 'react';
import { Plus, Trash2, Settings } from 'lucide-react';
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
  const [customKey, setCustomKey] = useState('');
  const [customValue, setCustomValue] = useState('');
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Get all provider keys so we can filter them out
  const allProviderKeys = useMemo(() => {
    const keys = new Set<string>();
    for (const provider of providerDefinitions) {
      for (const keyDef of provider.keys) {
        keys.add(keyDef.key);
      }
    }
    return keys;
  }, [providerDefinitions]);

  // Custom secrets = those not matching any provider key
  const customSecrets = useMemo(
    () => secrets.filter((s) => !allProviderKeys.has(s.key)),
    [secrets, allProviderKeys],
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

  return (
    <div className={`flex-1 overflow-y-auto ${className ?? ''}`}>
      <div className="mx-auto w-full max-w-5xl px-6 py-8 lg:px-10">


        {/* Custom Secrets Section */}
        <div className="rounded-2xl border border-border/70 bg-card p-6 shadow-sm">
          <div>
            <p className="text-xs font-semibold uppercase tracking-[0.3em] text-muted-foreground">
              Custom
            </p>
            <h3 className="mt-2 text-lg font-semibold text-foreground">Custom Secrets</h3>
            <p className="mt-1 text-sm text-muted-foreground">
              Add custom environment variables and secrets available to your agents.
            </p>
          </div>

          {loading ? (
            <div className="mt-6 text-sm text-muted-foreground">Loading…</div>
          ) : (
            <>
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
                        onClick={() => handleDelete(secret.key)}
                        className="inline-flex justify-end text-muted-foreground transition hover:text-destructive"
                      >
                        <Trash2 className="h-4 w-4" />
                      </button>
                    </div>
                  ))}
                </div>
              )}

              {customSecrets.length === 0 && (
                <div className="mt-6 text-sm text-muted-foreground">
                  No custom secrets configured. Use the form above to add one.
                </div>
              )}
            </>
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
