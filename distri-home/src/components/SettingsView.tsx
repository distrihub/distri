import { useState, useEffect } from 'react';
import { ConfigurationPanel } from '@distri/react';
import { useDistriHomeConfig, useDistriHomeNavigate } from '../DistriHomeProvider';
import { useApiKeys } from '../hooks/useApiKeys';
import { SecretsView } from './SecretsView';
import { CreditCard, KeyRound, Settings as SettingsIcon, LockIcon } from 'lucide-react';

export interface SettingsViewProps {
  /**
   * Optional custom class name
   */
  className?: string;
  /**
   * Active section (tab)
   */
  activeSection?: 'configuration' | 'account' | 'api-keys' | 'secrets';
  /**
   * Callback when section changes
   */
  onSectionChange?: (section: 'configuration' | 'account' | 'api-keys' | 'secrets') => void;
}

export function SettingsView({
  className,
  activeSection,
  onSectionChange,
}: SettingsViewProps) {
  const { enableApiKeys, enableAccountBilling } = useDistriHomeConfig();
  const navigate = useDistriHomeNavigate();

  // Use the useApiKeys hook for API key management
  const {
    keys,
    loading: keysLoading,
    error,
    createKey,
    revokeKey
  } = useApiKeys();

  const [label, setLabel] = useState('');
  const [creating, setCreating] = useState(false);
  const [newSecret, setNewSecret] = useState<string | null>(null);

  const [actionError, setActionError] = useState<string | null>(null);
  const setActiveSection = (section: 'configuration' | 'account' | 'api-keys' | 'secrets') => {
    onSectionChange?.(section);
  };



  const tabs = [
    { id: 'configuration' as const, label: 'Configuration', icon: SettingsIcon, href: 'settings' },
    ...(enableAccountBilling !== false ? [{ id: 'account' as const, label: 'Account & billing', icon: CreditCard, href: 'settings/account' }] : []),
    { id: 'secrets' as const, label: 'Secrets', icon: LockIcon, href: 'settings/secrets' },
    ...(enableApiKeys ? [{ id: 'api-keys' as const, label: 'API keys', icon: KeyRound, href: 'settings/api-keys' }] : []),
  ];

  const handleCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!label.trim()) return;
    setCreating(true);
    setActionError(null);
    try {
      const key = await createKey(label.trim());
      setNewSecret(key.key ?? null);
      setLabel('');
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to create API key';
      setActionError(message);
    } finally {
      setCreating(false);
    }
  };

  const handleRevoke = async (id: string) => {
    setActionError(null);
    try {
      await revokeKey(id);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to revoke API key';
      setActionError(message);
    }
  };

  const displayError = error || actionError;

  return (
    <div className={`flex-1 overflow-y-auto ${className ?? ''}`}>
      <div className="mx-auto flex w-full max-w-5xl flex-col px-6 py-8 lg:px-10">
        <div className="mb-6">
          <p className="text-xs font-semibold uppercase tracking-[0.3em] text-primary">Settings</p>
          <h2 className="mt-2 text-3xl font-semibold text-foreground">Account & access</h2>
          <p className="mt-2 text-sm text-muted-foreground">
            Manage API keys and cloud configuration.
          </p>
        </div>

        <div className="border-b border-border/60">
          <nav className="-mb-px flex flex-wrap gap-6 text-sm font-medium text-muted-foreground">
            {tabs.map(({ id, label: tabLabel, icon: Icon, href }) => (
              <button
                key={id}
                type="button"
                onClick={() => {
                  setActiveSection(id);
                  if (href) {
                    navigate(href);
                  }
                  navigate(href);
                }}
                className={`flex items-center gap-2 border-b-2 px-1 py-3 transition ${activeSection === id
                  ? 'border-primary text-primary'
                  : 'border-transparent hover:border-border/80 hover:text-foreground'
                  }`}
              >
                <Icon className="h-4 w-4" />
                {tabLabel}
              </button>
            ))}
          </nav>
        </div>

        <div className="mt-6">
          {activeSection === 'configuration' && (
            <div className="space-y-6">
              <div className="rounded-2xl border border-border/70 bg-card p-6 shadow-sm">
                <ConfigurationPanel title="Agent settings" />
              </div>
            </div>
          )}

          {activeSection === 'account' && (
            <div className="space-y-6">
              <div className="rounded-2xl border border-border/70 bg-card p-6 shadow-sm">
                <div>
                  <p className="text-xs font-semibold uppercase tracking-[0.3em] text-muted-foreground">
                    Account
                  </p>
                  <h3 className="mt-2 text-lg font-semibold text-foreground">Account & billing</h3>
                  <p className="mt-1 text-sm text-muted-foreground">
                    Manage your account settings and billing information.
                  </p>
                </div>
                <div className="mt-4 text-sm text-muted-foreground">
                  Account management coming soon.
                </div>
              </div>
            </div>
          )}

          {activeSection === 'api-keys' && enableApiKeys && (
            <div className="space-y-6">
              <div className="rounded-2xl border border-border/70 bg-card p-6 shadow-sm">
                <div>
                  <p className="text-xs font-semibold uppercase tracking-[0.3em] text-muted-foreground">
                    API keys
                  </p>
                  <h3 className="mt-2 text-lg font-semibold text-foreground">Create a new key</h3>
                  <p className="mt-1 text-sm text-muted-foreground">
                    Use API keys for backend and CLI access to your agents.
                  </p>
                </div>
                <form
                  onSubmit={handleCreate}
                  className="mt-5 grid gap-3 sm:grid-cols-[1fr_auto] sm:items-end"
                >
                  <div>
                    <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                      Label
                    </label>
                    <input
                      value={label}
                      onChange={(e) => setLabel(e.target.value)}
                      placeholder="Production key"
                      className="mt-2 h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground shadow-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                    />
                  </div>
                  <button
                    type="submit"
                    disabled={creating || !label.trim()}
                    className="inline-flex h-10 items-center justify-center rounded-md bg-primary px-4 text-sm font-semibold text-primary-foreground shadow-sm shadow-primary/20 transition hover:bg-primary/90 disabled:opacity-50"
                  >
                    {creating ? 'Creating…' : 'Create API key'}
                  </button>
                </form>
                {newSecret ? (
                  <div className="mt-4 rounded-lg border border-primary/40 bg-primary/5 p-4 text-sm">
                    <p className="font-semibold text-foreground">New key created</p>
                    <p className="mt-2 break-all text-muted-foreground">{newSecret}</p>
                    <p className="mt-2 text-xs text-muted-foreground">
                      Copy it now. It will not be shown again.
                    </p>
                  </div>
                ) : null}
              </div>

              <div className="overflow-hidden rounded-2xl border border-border/70 bg-card shadow-sm">
                <div className="border-b border-border/60 px-6 py-4">
                  <p className="text-sm font-semibold text-foreground">Existing keys</p>
                  <p className="text-xs text-muted-foreground">
                    Rotate or revoke keys used by backend clients.
                  </p>
                </div>
                {displayError ? (
                  <div className="border-b border-red-400/50 bg-red-500/10 px-6 py-3 text-sm text-red-700 dark:text-red-200">
                    {displayError}
                  </div>
                ) : null}
                <div className="divide-y divide-border/60">
                  {keysLoading ? (
                    <div className="px-6 py-4 text-sm text-muted-foreground">Loading…</div>
                  ) : keys.length === 0 ? (
                    <div className="px-6 py-4 text-sm text-muted-foreground">No API keys yet.</div>
                  ) : (
                    keys.map((key) => (
                      <div
                        key={key.id}
                        className="flex flex-wrap items-center justify-between gap-3 px-6 py-4"
                      >
                        <div className="min-w-0">
                          <p className="truncate text-sm font-semibold text-foreground">
                            {key.label || key.name || 'Untitled key'}
                          </p>
                          <p className="mt-1 text-xs text-muted-foreground">
                            Created{' '}
                            {key.created_at ? new Date(key.created_at).toLocaleString() : '—'}
                          </p>
                        </div>
                        <button
                          type="button"
                          onClick={() => handleRevoke(key.id)}
                          className="rounded-md border border-border/70 px-3 py-1.5 text-sm font-semibold text-foreground transition hover:border-primary/60 hover:text-primary"
                        >
                          Revoke
                        </button>
                      </div>
                    ))
                  )}
                </div>
              </div>
            </div>
          )}

          {activeSection === 'secrets' && (
            <SecretsView />
          )}
        </div>
      </div>
    </div>
  );
}
