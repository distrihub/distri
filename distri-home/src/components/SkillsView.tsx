import { useCallback, useEffect, useState } from 'react';
import { Code, Edit2, Plus, Trash2, X } from 'lucide-react';
import { useDistriHomeClient } from '../DistriHomeProvider';
import type { SkillRecord, SkillScriptRecord, NewSkill, NewSkillScript } from '../DistriHomeClient';

export interface SkillsViewProps {
  className?: string;
}

export function SkillsView({ className }: SkillsViewProps) {
  const homeClient = useDistriHomeClient();
  const [skills, setSkills] = useState<SkillRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editingSkill, setEditingSkill] = useState<SkillRecord | null>(null);
  const [showCreateForm, setShowCreateForm] = useState(false);

  // Create/Edit form state
  const [formName, setFormName] = useState('');
  const [formDescription, setFormDescription] = useState('');
  const [formContent, setFormContent] = useState('');
  const [formTags, setFormTags] = useState('');
  const [formScripts, setFormScripts] = useState<NewSkillScript[]>([]);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    if (!homeClient) return;
    setLoading(true);
    setError(null);
    try {
      const result = await homeClient.listSkills();
      setSkills(result ?? []);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Unable to load skills');
    } finally {
      setLoading(false);
    }
  }, [homeClient]);

  useEffect(() => {
    void load();
  }, [load]);

  const resetForm = () => {
    setFormName('');
    setFormDescription('');
    setFormContent('');
    setFormTags('');
    setFormScripts([]);
    setEditingSkill(null);
    setShowCreateForm(false);
  };

  const startEdit = (skill: SkillRecord) => {
    setEditingSkill(skill);
    setFormName(skill.name);
    setFormDescription(skill.description ?? '');
    setFormContent(skill.content);
    setFormTags(skill.tags.join(', '));
    setFormScripts(
      skill.scripts.map((s) => ({
        name: s.name,
        description: s.description,
        code: s.code,
        language: s.language,
      })),
    );
    setShowCreateForm(true);
  };

  const startCreate = () => {
    resetForm();
    setShowCreateForm(true);
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!homeClient || !formName.trim()) return;
    setSaving(true);
    setError(null);
    try {
      const tags = formTags
        .split(',')
        .map((t) => t.trim())
        .filter(Boolean);

      if (editingSkill) {
        await homeClient.updateSkill(editingSkill.id, {
          name: formName.trim(),
          description: formDescription.trim() || undefined,
          content: formContent,
          tags,
        });
      } else {
        const data: NewSkill = {
          name: formName.trim(),
          description: formDescription.trim() || undefined,
          content: formContent,
          tags,
          scripts: formScripts,
        };
        await homeClient.createSkill(data);
      }
      resetForm();
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Unable to save skill');
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    if (!homeClient) return;
    setError(null);
    try {
      await homeClient.deleteSkill(id);
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Unable to delete skill');
    }
  };

  const addScript = () => {
    setFormScripts((prev) => [
      ...prev,
      { name: '', code: '', language: 'javascript' },
    ]);
  };

  const updateScript = (index: number, updates: Partial<NewSkillScript>) => {
    setFormScripts((prev) =>
      prev.map((s, i) => (i === index ? { ...s, ...updates } : s)),
    );
  };

  const removeScript = (index: number) => {
    setFormScripts((prev) => prev.filter((_, i) => i !== index));
  };

  return (
    <div className={`flex-1 overflow-y-auto ${className ?? ''}`}>
      <div className="mx-auto w-full max-w-5xl px-6 py-8 lg:px-10">
        {/* Header */}
        <div className="rounded-2xl border border-border/70 bg-card p-6 shadow-sm">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-xs font-semibold uppercase tracking-[0.3em] text-muted-foreground">
                Skills
              </p>
              <h3 className="mt-2 text-lg font-semibold text-foreground">
                Agent Skills
              </h3>
              <p className="mt-1 text-sm text-muted-foreground">
                Create reusable skills with instructions and scripts that can be
                attached to agents.
              </p>
            </div>
            {!showCreateForm && (
              <button
                type="button"
                onClick={startCreate}
                className="inline-flex items-center gap-2 rounded-md bg-primary px-4 py-2 text-sm font-semibold text-primary-foreground transition hover:bg-primary/90"
              >
                <Plus className="h-4 w-4" />
                New Skill
              </button>
            )}
          </div>

          {/* Create/Edit Form */}
          {showCreateForm && (
            <form onSubmit={handleSave} className="mt-6 space-y-4">
              <div className="flex items-center justify-between">
                <h4 className="text-sm font-semibold text-foreground">
                  {editingSkill ? 'Edit Skill' : 'Create Skill'}
                </h4>
                <button
                  type="button"
                  onClick={resetForm}
                  className="text-muted-foreground hover:text-foreground"
                >
                  <X className="h-4 w-4" />
                </button>
              </div>

              <div className="grid gap-4 sm:grid-cols-2">
                <div>
                  <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                    Name
                  </label>
                  <input
                    value={formName}
                    onChange={(e) => setFormName(e.target.value)}
                    placeholder="my-skill"
                    required
                    className="mt-2 h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                  />
                </div>
                <div>
                  <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                    Tags
                  </label>
                  <input
                    value={formTags}
                    onChange={(e) => setFormTags(e.target.value)}
                    placeholder="tag1, tag2"
                    className="mt-2 h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                  />
                </div>
              </div>

              <div>
                <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                  Description
                </label>
                <input
                  value={formDescription}
                  onChange={(e) => setFormDescription(e.target.value)}
                  placeholder="What this skill does..."
                  className="mt-2 h-10 w-full rounded-md border border-border/70 bg-background px-3 text-sm text-foreground focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                />
              </div>

              <div>
                <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                  Content (Instructions)
                </label>
                <textarea
                  value={formContent}
                  onChange={(e) => setFormContent(e.target.value)}
                  placeholder="Markdown instructions that will be injected into the agent's system prompt..."
                  rows={6}
                  className="mt-2 w-full rounded-md border border-border/70 bg-background px-3 py-2 font-mono text-sm text-foreground focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                />
              </div>

              {/* Scripts Section */}
              <div>
                <div className="flex items-center justify-between">
                  <label className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">
                    Scripts
                  </label>
                  <button
                    type="button"
                    onClick={addScript}
                    className="inline-flex items-center gap-1 text-xs text-primary hover:text-primary/80"
                  >
                    <Plus className="h-3 w-3" />
                    Add Script
                  </button>
                </div>

                {formScripts.map((script, index) => (
                  <div
                    key={index}
                    className="mt-3 rounded-lg border border-border/60 bg-muted/20 p-4"
                  >
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-2">
                        <Code className="h-4 w-4 text-muted-foreground" />
                        <span className="text-xs font-semibold text-muted-foreground">
                          Script {index + 1}
                        </span>
                      </div>
                      <button
                        type="button"
                        onClick={() => removeScript(index)}
                        className="text-muted-foreground hover:text-destructive"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                    <div className="mt-3 grid gap-3 sm:grid-cols-2">
                      <input
                        value={script.name}
                        onChange={(e) =>
                          updateScript(index, { name: e.target.value })
                        }
                        placeholder="Script name"
                        className="h-9 rounded-md border border-border/70 bg-background px-3 text-sm text-foreground focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                      />
                      <select
                        value={script.language ?? 'javascript'}
                        onChange={(e) =>
                          updateScript(index, { language: e.target.value })
                        }
                        className="h-9 rounded-md border border-border/70 bg-background px-3 text-sm text-foreground focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                      >
                        <option value="javascript">JavaScript</option>
                        <option value="python">Python</option>
                        <option value="bash">Bash</option>
                        <option value="typescript">TypeScript</option>
                      </select>
                    </div>
                    <textarea
                      value={script.code}
                      onChange={(e) =>
                        updateScript(index, { code: e.target.value })
                      }
                      placeholder="// Script code..."
                      rows={4}
                      className="mt-3 w-full rounded-md border border-border/70 bg-background px-3 py-2 font-mono text-sm text-foreground focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                    />
                  </div>
                ))}
              </div>

              <div className="flex gap-3">
                <button
                  type="submit"
                  disabled={saving || !formName.trim()}
                  className="inline-flex items-center gap-2 rounded-md bg-primary px-4 py-2 text-sm font-semibold text-primary-foreground transition hover:bg-primary/90 disabled:opacity-50"
                >
                  {saving
                    ? 'Saving...'
                    : editingSkill
                      ? 'Update Skill'
                      : 'Create Skill'}
                </button>
                <button
                  type="button"
                  onClick={resetForm}
                  className="inline-flex items-center rounded-md border border-border/70 px-4 py-2 text-sm font-medium text-muted-foreground transition hover:text-foreground"
                >
                  Cancel
                </button>
              </div>
            </form>
          )}
        </div>

        {/* Skills List */}
        {loading ? (
          <div className="mt-6 text-sm text-muted-foreground">Loading...</div>
        ) : skills.length > 0 ? (
          <div className="mt-6 space-y-3">
            {skills.map((skill) => (
              <div
                key={skill.id}
                className="rounded-xl border border-border/70 bg-card p-4 shadow-sm"
              >
                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <div className="flex items-center gap-2">
                      <h4 className="text-sm font-semibold text-foreground">
                        {skill.name}
                      </h4>
                      {skill.tags.length > 0 && (
                        <div className="flex gap-1">
                          {skill.tags.map((tag) => (
                            <span
                              key={tag}
                              className="rounded-full bg-primary/10 px-2 py-0.5 text-xs text-primary"
                            >
                              {tag}
                            </span>
                          ))}
                        </div>
                      )}
                    </div>
                    {skill.description && (
                      <p className="mt-1 text-sm text-muted-foreground">
                        {skill.description}
                      </p>
                    )}
                    {skill.scripts.length > 0 && (
                      <div className="mt-2 flex items-center gap-1 text-xs text-muted-foreground">
                        <Code className="h-3 w-3" />
                        {skill.scripts.length} script
                        {skill.scripts.length !== 1 ? 's' : ''}
                      </div>
                    )}
                  </div>
                  <div className="flex items-center gap-2">
                    <button
                      type="button"
                      onClick={() => startEdit(skill)}
                      className="text-muted-foreground transition hover:text-foreground"
                      title="Edit"
                    >
                      <Edit2 className="h-4 w-4" />
                    </button>
                    <button
                      type="button"
                      onClick={() => handleDelete(skill.id)}
                      className="text-muted-foreground transition hover:text-destructive"
                      title="Delete"
                    >
                      <Trash2 className="h-4 w-4" />
                    </button>
                  </div>
                </div>
              </div>
            ))}
          </div>
        ) : (
          !showCreateForm && (
            <div className="mt-6 rounded-xl border border-dashed border-border/70 p-8 text-center">
              <p className="text-sm text-muted-foreground">
                No skills yet. Create your first skill to get started.
              </p>
            </div>
          )
        )}

        {error && (
          <div className="mt-6 rounded-xl border border-red-400/50 bg-red-500/10 px-4 py-3 text-sm text-red-700 dark:text-red-200">
            {error}
          </div>
        )}
      </div>
    </div>
  );
}
