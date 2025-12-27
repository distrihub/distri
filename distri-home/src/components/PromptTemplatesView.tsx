import { useCallback, useEffect, useMemo, useState } from 'react';
import { AlertTriangle, Copy, FileText, Filter, Lock, Plus, Save, Search, Trash2, X } from 'lucide-react';
import { useDistriHomeClient } from '../DistriHomeProvider';

// Types
export interface PromptTemplate {
  id: string;
  name: string;
  template: string;
  description?: string;
  version?: string;
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

  // Selection and editing
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [isCreating, setIsCreating] = useState(false);
  const [name, setName] = useState('');
  const [template, setTemplate] = useState('');

  // Filters
  const [query, setQuery] = useState('');
  const [showSystemOnly, setShowSystemOnly] = useState(false);
  const [showFilterMenu, setShowFilterMenu] = useState(false);

  const secretKeys = useMemo(
    () => new Set(secrets.map((secret) => secret.key.toUpperCase())),
    [secrets],
  );

  const missingSecrets = useMemo(() => {
    const referenced = extractSecretKeys(template);
    return referenced.filter((key) => !secretKeys.has(key));
  }, [template, secretKeys]);

  // Filtered templates
  const filteredTemplates = useMemo(() => {
    return templates.filter((t) => {
      const matchesQuery = query
        ? (t.name.toLowerCase().includes(query.toLowerCase()) ||
          t.template.toLowerCase().includes(query.toLowerCase()))
        : true;
      const matchesSource = showSystemOnly ? t.is_system : true;
      return matchesQuery && matchesSource;
    });
  }, [templates, query, showSystemOnly]);

  const selectedTemplate = useMemo(
    () => templates.find(t => t.id === selectedId),
    [templates, selectedId]
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
      const message = err instanceof Error ? err.message : 'Unable to load templates';
      setError(message);
    } finally {
      setLoading(false);
    }
  }, [homeClient]);

  useEffect(() => {
    void load();
  }, [load]);

  // Effect to sync state with URL query parameter
  useEffect(() => {
    const url = new URL(window.location.href);
    if (selectedId) {
      url.searchParams.set('id', selectedId);
    } else {
      url.searchParams.delete('id');
    }
    window.history.pushState({}, '', url.toString());
  }, [selectedId]);

  // Handle initial selection from URL and sync with loaded templates
  useEffect(() => {
    if (!loading && templates.length > 0) {
      const searchParams = new URLSearchParams(window.location.search);
      const idFromUrl = searchParams.get('id');

      if (idFromUrl && idFromUrl !== selectedId) {
        const t = templates.find(t => t.id === idFromUrl);
        if (t) {
          setSelectedId(idFromUrl);
          setIsCreating(false);
          setName(t.name);
          setTemplate(t.template);
        }
      }
    }
  }, [loading, templates, selectedId]);

  const selectTemplate = (id: string) => {
    const t = templates.find(t => t.id === id);
    if (t) {
      setSelectedId(id);
      setIsCreating(false);
      setName(t.name);
      setTemplate(t.template);
    }
  };

  const startCreate = () => {
    setSelectedId(null);
    setIsCreating(true);
    setName('');
    setTemplate('');
  };

  const cancelEdit = () => {
    setSelectedId(null);
    setIsCreating(false);
    setName('');
    setTemplate('');
  };

  const handleSave = async () => {
    if (!homeClient) return;
    if (!name.trim() || !template.trim()) return;
    setSaving(true);
    setError(null);
    try {
      let savedId = selectedId;
      if (selectedId && !isCreating) {
        await homeClient.updatePromptTemplate(selectedId, name.trim(), template.trim());
      } else {
        const created = await homeClient.createPromptTemplate(name.trim(), template.trim());
        savedId = created.id;
      }

      await load();

      // If we saved/created, ensure it's still selected
      if (savedId) {
        const t = templates.find(t => t.id === savedId);
        // Note: we might need to wait for templates state to update or use the returned value
        // But since we call load() and then templates changes, useEffect above will handle sync if id matches
        setSelectedId(savedId);
        setIsCreating(false);
      } else {
        cancelEdit();
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to save template';
      setError(message);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    if (!homeClient) return;
    const tpl = templates.find(t => t.id === id);
    if (!tpl) return;

    if (tpl.is_system) {
      setError('Cannot delete system templates');
      return;
    }
    if (!window.confirm(`Are you sure you want to delete "${tpl.name}"?`)) {
      return;
    }

    setError(null);
    try {
      await homeClient.deletePromptTemplate(id);
      if (selectedId === id) {
        cancelEdit();
      }
      await load();
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to delete template';
      setError(message);
    }
  };

  const handleClone = async (id: string) => {
    if (!homeClient) return;
    setLoading(true);
    setError(null);
    try {
      const cloned = await homeClient.clonePromptTemplate(id);
      await load();
      selectTemplate(cloned.id);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unable to clone template';
      setError(message);
    } finally {
      setLoading(false);
    }
  };

  const isEditing = selectedId !== null || isCreating;
  const canEdit = isCreating || (selectedTemplate && !selectedTemplate.is_system);

  return (
    <div className={`flex flex-1 h-full overflow-hidden bg-background ${className ?? ''}`}>
      {/* Left Panel - Template List */}
      <div className="w-72 shrink-0 flex flex-col border-r border-border/60 bg-card/30">
        {/* List Header */}
        <div className="flex items-center gap-2 border-b border-border/60 px-3 py-3">
          <div className="relative flex-1">
            <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search..."
              className="h-8 w-full rounded-md border border-border/70 bg-background pl-8 pr-2 text-xs text-foreground focus:border-primary focus:outline-none"
            />
          </div>
          <div className="relative">
            <button
              type="button"
              onClick={() => setShowFilterMenu(!showFilterMenu)}
              title="Filter options"
              className={`flex h-8 w-8 items-center justify-center rounded-md border transition ${showSystemOnly ? 'border-primary bg-primary/10 text-primary' : 'border-border/70 text-muted-foreground hover:text-foreground'}`}
            >
              <Filter className="h-3.5 w-3.5" />
            </button>
            {showFilterMenu && (
              <div className="absolute right-0 top-full z-10 mt-1 w-40 rounded-md border border-border/70 bg-card p-2 shadow-lg">
                <label className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-xs hover:bg-muted/50">
                  <input
                    type="checkbox"
                    checked={showSystemOnly}
                    onChange={(e) => setShowSystemOnly(e.target.checked)}
                    className="h-3.5 w-3.5 rounded border-border accent-primary"
                  />
                  System only
                </label>
              </div>
            )}
          </div>
          <button
            type="button"
            onClick={startCreate}
            title="New template"
            className="flex h-8 w-8 items-center justify-center rounded-md bg-primary text-primary-foreground transition hover:bg-primary/90"
          >
            <Plus className="h-4 w-4" />
          </button>
        </div>

        {/* Template List */}
        <div className="flex-1 overflow-y-auto">
          {loading ? (
            <div className="px-3 py-4 text-xs text-muted-foreground">Loading…</div>
          ) : filteredTemplates.length === 0 ? (
            <div className="px-3 py-4 text-xs text-muted-foreground">No templates found.</div>
          ) : (
            <div className="py-1">
              {filteredTemplates.map((tpl) => (
                <button
                  key={tpl.id}
                  type="button"
                  onClick={() => selectTemplate(tpl.id)}
                  className={`group w-full px-3 py-2.5 text-left transition hover:bg-muted/50 ${selectedId === tpl.id ? 'bg-primary/10 border-l-2 border-l-primary' : 'border-l-2 border-l-transparent'
                    }`}
                >
                  <div className="flex items-start gap-2">
                    <div className="mt-0.5 shrink-0">
                      {tpl.is_system ? (
                        <Lock className="h-3.5 w-3.5 text-amber-500" />
                      ) : (
                        <FileText className="h-3.5 w-3.5 text-muted-foreground" />
                      )}
                    </div>
                    <div className="min-w-0 flex-1">
                      <p className="text-sm font-medium text-foreground leading-tight">
                        {tpl.name}
                      </p>
                      <p className="mt-0.5 text-[11px] text-muted-foreground/80 font-mono truncate">
                        {tpl.template.slice(0, 50)}{tpl.template.length > 50 ? '…' : ''}
                      </p>
                    </div>
                  </div>
                </button>
              ))}
            </div>
          )}
        </div>

        {/* List Footer */}
        <div className="border-t border-border/60 px-3 py-2 text-[10px] text-muted-foreground">
          {templates.filter(t => t.is_system).length} system · {templates.filter(t => !t.is_system).length} user
        </div>
      </div>

      {/* Right Panel - Editor */}
      <div className="flex flex-1 flex-col overflow-hidden">
        {!isEditing ? (
          <div className="flex flex-1 items-center justify-center">
            <div className="text-center">
              <FileText className="mx-auto h-10 w-10 text-muted-foreground/30" />
              <p className="mt-3 text-sm text-muted-foreground">Select a template</p>
            </div>
          </div>
        ) : (
          <>
            {/* Editor Toolbar */}
            <div className="flex items-center gap-3 border-b border-border/60 px-4 py-2">
              {/* Status badges */}
              <div className="flex shrink-0 items-center gap-2">
                {selectedTemplate?.is_system && (
                  <span className="inline-flex items-center gap-1 rounded bg-amber-500/10 px-2 py-0.5 text-[10px] font-semibold text-amber-600 dark:text-amber-400">
                    <Lock className="h-3 w-3" />
                    READ-ONLY
                  </span>
                )}
                {isCreating && (
                  <span className="inline-flex items-center gap-1 rounded bg-primary/10 px-2 py-0.5 text-[10px] font-semibold text-primary">
                    <Plus className="h-3 w-3" />
                    NEW
                  </span>
                )}
              </div>

              {/* Name Input */}
              <input
                value={name}
                onChange={(e) => setName(e.target.value)}
                disabled={selectedTemplate?.is_system}
                placeholder="Template name..."
                className="h-9 flex-1 rounded-md border border-border/60 bg-background px-3 text-sm font-medium text-foreground placeholder:text-muted-foreground/50 focus:border-primary focus:outline-none disabled:cursor-not-allowed disabled:bg-muted/30"
              />

              {/* Action buttons */}
              <div className="flex shrink-0 items-center gap-1">
                {canEdit && !selectedTemplate?.is_system && (
                  <>
                    <button
                      type="button"
                      onClick={handleSave}
                      disabled={saving || !name.trim() || !template.trim()}
                      title="Save"
                      className="flex h-8 w-8 items-center justify-center rounded-md bg-primary text-primary-foreground transition hover:bg-primary/90 disabled:opacity-50"
                    >
                      <Save className="h-4 w-4" />
                    </button>
                    <button
                      type="button"
                      onClick={cancelEdit}
                      title="Cancel"
                      className="flex h-8 w-8 items-center justify-center rounded-md border border-border/70 text-muted-foreground transition hover:text-foreground"
                    >
                      <X className="h-4 w-4" />
                    </button>
                  </>
                )}
                {selectedId && (
                  <button
                    type="button"
                    onClick={() => handleClone(selectedId)}
                    disabled={loading || saving}
                    title="Clone"
                    className="flex h-8 w-8 items-center justify-center rounded-md border border-border/70 text-muted-foreground transition hover:text-foreground disabled:opacity-50"
                  >
                    <Copy className="h-4 w-4" />
                  </button>
                )}
                {selectedId && !selectedTemplate?.is_system && (
                  <button
                    type="button"
                    onClick={() => handleDelete(selectedId)}
                    disabled={loading || saving}
                    title="Delete"
                    className="flex h-8 w-8 items-center justify-center rounded-md border border-red-400/50 text-red-500 transition hover:bg-red-500/10 disabled:opacity-50"
                  >
                    <Trash2 className="h-4 w-4" />
                  </button>
                )}
              </div>
            </div>

            {/* Error */}
            {error && (
              <div className="mx-4 mt-3 flex items-center gap-2 rounded-md border border-red-400/50 bg-red-500/10 px-3 py-2 text-xs text-red-600 dark:text-red-300">
                <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
                {error}
              </div>
            )}

            {/* Editor Content */}
            <div className="flex flex-1 flex-col overflow-hidden p-4">
              {/* Template Editor */}
              <textarea
                value={template}
                onChange={(e) => setTemplate(e.target.value)}
                disabled={selectedTemplate?.is_system}
                placeholder="Write your prompt template here...&#10;&#10;Use {{secrets.KEY}} for secrets"
                className="min-h-0 flex-1 resize-none rounded-lg border border-border/60 bg-muted/20 p-4 font-mono text-sm text-foreground placeholder:text-muted-foreground/50 focus:border-primary focus:outline-none disabled:cursor-not-allowed disabled:bg-muted/40"
              />

              {/* Missing Secrets Warning */}
              {missingSecrets.length > 0 && (
                <div className="mt-3 flex items-center gap-2 text-xs text-amber-600 dark:text-amber-400">
                  <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
                  Missing: <span className="font-mono font-semibold">{missingSecrets.join(', ')}</span>
                </div>
              )}

              {/* Metadata */}
              {selectedTemplate && (
                <div className="mt-4 flex flex-wrap items-center gap-3 text-[11px] text-muted-foreground">
                  <span>Type: <b>{selectedTemplate.is_system ? 'System' : 'User'}</b></span>
                  {selectedTemplate.version && <span>Version: <b>v{selectedTemplate.version}</b></span>}
                  {selectedTemplate.updated_at && (
                    <span>Updated: <b>{new Date(selectedTemplate.updated_at).toLocaleDateString()}</b></span>
                  )}
                </div>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
