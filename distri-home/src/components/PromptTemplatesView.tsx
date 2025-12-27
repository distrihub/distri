import { useCallback, useEffect, useMemo, useState } from 'react';
import { Lock, Pencil, Plus, Save, Trash2, X } from 'lucide-react';
import { useDistriHomeClient } from '../DistriHomeProvider';

// Types
export interface PromptTemplate {
  id: string;
  name: string;
  template: string;
  description?: string;
  version?: string;
  source?: 'static' | 'file' | 'dynamic' | 'user';
  is_system?: boolean;
  created_at?: string;
  updated_at?: string;
}

export interface Secret {
  id: string;
  key: string;
  masked_value: string;
}

const SECRET_TOKEN_PATTERN = /\{\{\s*secrets\.([A-Z0-9_]+)\s*\}\}/gi;

function extractSecretKeys(template: string) {
  const keys = new Set<string>();
  let match: RegExpExecArray | null;
  SECRET_TOKEN_PATTERN.lastIndex = 0;
  while ((match = SECRET_TOKEN_PATTERN.exec(template)) !== null) {
    keys.add(match[1].toUpperCase());
  }
  return Array.from(keys);
}

export interface PromptTemplatesViewProps {
  className?: string;
}

export function PromptTemplatesView({ className }: PromptTemplatesViewProps) {
  const homeClient = useDistriHomeClient();
  const [templates, setTemplates] = useState<PromptTemplate[]>([]);
  const [secrets, setSecrets] = useState<Secret[]>([]);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [template, setTemplate] = useState('');

  const secretKeys = useMemo(
    () => new Set(secrets.map((secret) => secret.key.toUpperCase())),
    [secrets],
  );

  const missingSecrets = useMemo(() => {
    const referenced = extractSecretKeys(template);
    return referenced.filter((key) => !secretKeys.has(key));
  }, [template, secretKeys]);

  // Separate system and user templates
  const systemTemplates = useMemo(
    () => templates.filter((t) => t.is_system),
    [templates],
  );

  const userTemplates = useMemo(
    () => templates.filter((t) => !t.is_system),
    [templates],
  );

  const load = useCallback(async () => {
    if (!homeClient) return;
    setLoading(true);
    setError(null);
    try {
      const [templateResponse, secretResponse] = await Promise.all([
        homeClient.listPromptTemplates(),
        homeClient.listSecrets(),
      ]);
      setTemplates(templateResponse ?? []);
      setSecrets(secretResponse ?? []);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to load prompt templates';
      setError(message);
    } finally {
      setLoading(false);
    }
  }, [homeClient]);

  useEffect(() => {
    void load();
  }, [load]);

  const resetForm = () => {
    setEditingId(null);
    setName('');
    setTemplate('');
  };

  const startEdit = (item: PromptTemplate) => {
    if (item.is_system) return; // Can't edit system templates
    setEditingId(item.id);
    setName(item.name);
    setTemplate(item.template);
  };

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!homeClient) return;
    if (!name.trim() || !template.trim()) return;
    setSaving(true);
    setError(null);
    try {
      if (editingId) {
        await homeClient.updatePromptTemplate(editingId, name.trim(), template.trim());
      } else {
        await homeClient.createPromptTemplate(name.trim(), template.trim());
      }
      resetForm();
      await load();
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to save prompt template';
      setError(message);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    if (!homeClient) return;
    const templateToDelete = templates.find((t) => t.id === id);
    if (templateToDelete?.is_system) {
      setError('Cannot delete system templates');
      return;
    }
    setError(null);
    try {
      await homeClient.deletePromptTemplate(id);
      await load();
      if (editingId === id) {
        resetForm();
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to delete prompt template';
      setError(message);
    }
  };

  return (
    <div className={`flex-1 overflow-y-auto ${className ?? ''}`}>
      <div className="mx-auto w-full max-w-5xl px-6 py-8 lg:px-10">
        {/* Header */}
        <div className="mb-6">
          <p className="text-xs font-semibold uppercase tracking-[0.3em] text-primary">Templates</p>
          <h2 className="mt-2 text-3xl font-semibold text-foreground">Prompt Templates</h2>
          <p className="mt-2 text-sm text-muted-foreground">
            Manage reusable prompt templates. Use <span className="font-semibold">{"{{secrets.KEY}}"}</span>{' '}
            placeholders for stored secrets. System templates are locked.
          </p>
        </div>

        {/* Create/Edit Form */}
        <div className="rounded-2xl border border-border/70 bg-card p-6 shadow-sm">
          <div className="flex flex-wrap items-center justify-between gap-4">
            <div>
              <h3 className="text-lg font-semibold text-foreground">
                {editingId ? 'Edit template' : 'Create template'}
              </h3>
            </div>
            <div className="text-xs font-semibold uppercase tracking-[0.2em] text-muted-foreground">
              {userTemplates.length} user / {systemTemplates.length} system
            </div>
          </div>

          <form onSubmit={handleSubmit} className="mt-6 space-y-4">
            <div className="grid gap-4 lg:grid-cols-[260px_1fr]">
              <div>
                <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                  Name
                </label>
                <input
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  placeholder="Summarize customer request"
                  className="mt-2 h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground shadow-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                />
              </div>
              <div className="flex items-end justify-end gap-2">
                {editingId ? (
                  <button
                    type="button"
                    onClick={resetForm}
                    className="inline-flex h-10 items-center justify-center gap-2 rounded-md border border-border/70 px-4 text-sm font-semibold text-muted-foreground transition hover:text-foreground"
                  >
                    <X className="h-4 w-4" />
                    Cancel
                  </button>
                ) : null}
                <button
                  type="submit"
                  disabled={saving || !name.trim() || !template.trim()}
                  className="inline-flex h-10 items-center justify-center gap-2 rounded-md bg-primary px-4 text-sm font-semibold text-primary-foreground shadow-sm shadow-primary/20 transition hover:bg-primary/90 disabled:opacity-50"
                >
                  {editingId ? <Save className="h-4 w-4" /> : <Plus className="h-4 w-4" />}
                  {editingId ? 'Save template' : 'Add template'}
                </button>
              </div>
            </div>

            <div>
              <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                Template
              </label>
              <textarea
                value={template}
                onChange={(event) => setTemplate(event.target.value)}
                placeholder="You can use secrets using {{secrets.CUSTOM_KEY}} if needed."
                className="mt-2 min-h-[140px] w-full rounded-md border border-border/70 bg-background px-3 py-2 text-sm text-foreground shadow-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
              />
              {missingSecrets.length > 0 ? (
                <p className="mt-2 text-xs font-semibold text-amber-500">
                  Missing secrets: {missingSecrets.join(', ')}
                </p>
              ) : null}
            </div>
          </form>
        </div>

        {/* Error message */}
        {error ? (
          <div className="mt-6 rounded-xl border border-red-400/50 bg-red-500/10 px-4 py-3 text-sm text-red-700 dark:text-red-200">
            {error}
          </div>
        ) : null}

        {/* System Templates */}
        {systemTemplates.length > 0 && (
          <div className="mt-8 overflow-hidden rounded-2xl border border-border/70 bg-card shadow-sm">
            <div className="border-b border-border/60 px-6 py-4">
              <div className="flex items-center gap-2">
                <Lock className="h-4 w-4 text-muted-foreground" />
                <p className="text-sm font-semibold text-foreground">System Templates</p>
              </div>
              <p className="text-xs text-muted-foreground">Built-in templates (read-only)</p>
            </div>
            <div className="divide-y divide-border/60">
              {systemTemplates.map((item) => (
                <div key={item.id} className="flex flex-wrap items-center justify-between gap-4 px-6 py-4">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <p className="truncate text-sm font-semibold text-foreground">{item.name}</p>
                      <Lock className="h-3 w-3 text-muted-foreground" />
                    </div>
                    {item.description && (
                      <p className="mt-1 truncate text-xs text-muted-foreground">{item.description}</p>
                    )}
                    <p className="mt-1 truncate text-xs font-mono text-muted-foreground/70">
                      {item.template.slice(0, 100)}{item.template.length > 100 ? '...' : ''}
                    </p>
                  </div>
                  <div className="flex items-center gap-2 text-xs text-muted-foreground">
                    {item.version && <span>v{item.version}</span>}
                    <span className="rounded border border-border/60 bg-muted/50 px-2 py-0.5">{item.source}</span>
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* User Templates */}
        <div className="mt-8 overflow-hidden rounded-2xl border border-border/70 bg-card shadow-sm">
          <div className="border-b border-border/60 px-6 py-4">
            <p className="text-sm font-semibold text-foreground">Your Templates</p>
            <p className="text-xs text-muted-foreground">Create and manage custom templates</p>
          </div>
          <div className="divide-y divide-border/60">
            {loading ? (
              <div className="px-6 py-4 text-sm text-muted-foreground">Loading…</div>
            ) : userTemplates.length === 0 ? (
              <div className="px-6 py-4 text-sm text-muted-foreground">No custom templates yet.</div>
            ) : (
              userTemplates.map((item) => (
                <div key={item.id} className="flex flex-wrap items-center justify-between gap-4 px-6 py-4">
                  <div className="min-w-0">
                    <p className="truncate text-sm font-semibold text-foreground">{item.name}</p>
                    <p className="mt-1 truncate text-xs text-muted-foreground">{item.template}</p>
                  </div>
                  <div className="flex items-center gap-2">
                    <button
                      type="button"
                      onClick={() => startEdit(item)}
                      className="inline-flex items-center gap-2 rounded-md border border-border/70 px-3 py-1.5 text-sm font-semibold text-muted-foreground transition hover:text-foreground"
                    >
                      <Pencil className="h-3.5 w-3.5" />
                      Edit
                    </button>
                    <button
                      type="button"
                      onClick={() => handleDelete(item.id)}
                      className="inline-flex items-center gap-2 rounded-md border border-border/70 px-3 py-1.5 text-sm font-semibold text-muted-foreground transition hover:text-destructive"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                      Delete
                    </button>
                  </div>
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
