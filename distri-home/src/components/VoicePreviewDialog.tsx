import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import * as SelectPrimitive from '@radix-ui/react-select';
import { Check, ChevronDown, Copy, Globe, Loader2, Search, Volume2, X } from 'lucide-react';
import { DistriHomeClient, ModelProviderDefinition } from '../DistriHomeClient';
import type { TtsVoiceInfo } from '../DistriHomeClient';
import { cn } from '../lib/utils';

interface VoicePreviewDialogProps {
  open: boolean;
  onClose: () => void;
  providers: ModelProviderDefinition[];
  isProviderConfigured: (provider: ModelProviderDefinition) => boolean;
  homeClient: DistriHomeClient;
  onOpenProviders?: () => void;
  initialModelId?: string;
  initialProviderId?: string;
}

interface VoiceEntry {
  voice: TtsVoiceInfo;
  providerId: string;
  providerLabel: string;
  modelId: string;
  modelLabel: string;
  key: string;
}

interface ModelOption {
  providerId: string;
  providerLabel: string;
  modelId: string;
  modelLabel: string;
  voiceCount: number;
}

const SAMPLE_TEXT = 'Hello, this is a preview of how this voice sounds.';

function voiceMatches(voice: TtsVoiceInfo, query: string): boolean {
  if (!query) return true;
  const q = query.toLowerCase();
  return (
    voice.name.toLowerCase().includes(q) ||
    voice.id.toLowerCase().includes(q) ||
    (voice.description?.toLowerCase().includes(q) ?? false) ||
    (voice.languages?.some((l) => l.toLowerCase().includes(q)) ?? false)
  );
}

/* ── Thin Radix Select wrappers (same as shadcn) ──────────────────────────── */

function Select({ ...props }: React.ComponentPropsWithoutRef<typeof SelectPrimitive.Root>) {
  return <SelectPrimitive.Root {...props} />;
}

function SelectTrigger({
  className,
  children,
  ...props
}: React.ComponentPropsWithoutRef<typeof SelectPrimitive.Trigger>) {
  return (
    <SelectPrimitive.Trigger
      className={cn(
        'flex h-9 w-full items-center justify-between whitespace-nowrap rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm ring-offset-background data-[placeholder]:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring disabled:cursor-not-allowed disabled:opacity-50 [&>span]:line-clamp-1',
        className,
      )}
      {...props}
    >
      {children}
      <SelectPrimitive.Icon asChild>
        <ChevronDown className="h-4 w-4 opacity-50" />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
}

function SelectContent({
  className,
  children,
  position = 'popper',
  ...props
}: React.ComponentPropsWithoutRef<typeof SelectPrimitive.Content>) {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Content
        className={cn(
          'relative z-50 max-h-[--radix-select-content-available-height] min-w-[8rem] overflow-y-auto overflow-x-hidden rounded-md border bg-popover text-popover-foreground shadow-md data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 origin-[--radix-select-content-transform-origin]',
          position === 'popper' &&
            'data-[side=bottom]:translate-y-1 data-[side=left]:-translate-x-1 data-[side=right]:translate-x-1 data-[side=top]:-translate-y-1',
          className,
        )}
        position={position}
        {...props}
      >
        <SelectPrimitive.Viewport
          className={cn(
            'p-1',
            position === 'popper' &&
              'h-[var(--radix-select-trigger-height)] w-full min-w-[var(--radix-select-trigger-width)]',
          )}
        >
          {children}
        </SelectPrimitive.Viewport>
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  );
}

function SelectItem({
  className,
  children,
  ...props
}: React.ComponentPropsWithoutRef<typeof SelectPrimitive.Item>) {
  return (
    <SelectPrimitive.Item
      className={cn(
        'relative flex w-full cursor-default select-none items-center rounded-sm py-1.5 pl-2 pr-8 text-sm outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50',
        className,
      )}
      {...props}
    >
      <span className="absolute right-2 flex h-3.5 w-3.5 items-center justify-center">
        <SelectPrimitive.ItemIndicator>
          <Volume2 className="h-4 w-4" />
        </SelectPrimitive.ItemIndicator>
      </span>
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
    </SelectPrimitive.Item>
  );
}

/* ── Dialog component ─────────────────────────────────────────────────────── */

export function VoicePreviewDialog({
  open,
  onClose,
  providers,
  isProviderConfigured,
  homeClient,
  onOpenProviders,
  initialModelId,
  initialProviderId,
}: VoicePreviewDialogProps) {
  const [searchQuery, setSearchQuery] = useState('');
  const [previewingVoice, setPreviewingVoice] = useState<string | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const audioRef = useRef<HTMLAudioElement | null>(null);

  // Build list of all available TTS model options
  const modelOptions: ModelOption[] = useMemo(() => {
    const opts: ModelOption[] = [];
    for (const provider of providers) {
      if (!isProviderConfigured(provider)) continue;
      for (const model of provider.models) {
        if (model.capability !== 'tts' || !model.voices?.length) continue;
        opts.push({
          providerId: provider.id,
          providerLabel: provider.label,
          modelId: model.id,
          modelLabel: model.name,
          voiceCount: model.voices.length,
        });
      }
    }
    return opts;
  }, [providers, isProviderConfigured]);

  // Selected model as a `providerId:modelId` string
  const defaultModelKey = useMemo(() => {
    if (initialModelId && initialProviderId) return `${initialProviderId}:${initialModelId}`;
    if (initialModelId) {
      const found = modelOptions.find((o) => o.modelId === initialModelId);
      return found ? `${found.providerId}:${found.modelId}` : modelOptions[0] ? `${modelOptions[0].providerId}:${modelOptions[0].modelId}` : undefined;
    }
    return modelOptions[0] ? `${modelOptions[0].providerId}:${modelOptions[0].modelId}` : undefined;
  }, [initialModelId, initialProviderId, modelOptions]);

  const [modelKey, setModelKey] = useState<string | undefined>(undefined);

  // Use defaultModelKey on first render, then track user selection
  const resolvedModelKey = modelKey ?? defaultModelKey;

  const selected = useMemo(
    () => modelOptions.find((o) => `${o.providerId}:${o.modelId}` === resolvedModelKey) ?? null,
    [modelOptions, resolvedModelKey],
  );

  // Collect voices for the selected model
  const voices: VoiceEntry[] = useMemo(() => {
    if (!selected) return [];
    const provider = providers.find((p) => p.id === selected.providerId);
    if (!provider) return [];
    const model = provider.models.find((m) => m.id === selected.modelId);
    if (!model?.voices) return [];
    return model.voices.map((voice) => ({
      voice,
      providerId: provider.id,
      providerLabel: provider.label,
      modelId: model.id,
      modelLabel: model.name,
      key: `${provider.id}:${model.id}:${voice.id}`,
    }));
  }, [selected, providers]);

  // All unique languages for this model
  const allLanguages = useMemo(() => {
    const langSet = new Set<string>();
    for (const v of voices) {
      if (v.voice.languages) {
        for (const lang of v.voice.languages) langSet.add(lang);
      }
    }
    return Array.from(langSet).sort();
  }, [voices]);

  const [selectedLanguage, setSelectedLanguage] = useState<string>('__all__');

  // Reset filters when model changes
  useEffect(() => {
    setSearchQuery('');
    setPreviewingVoice(null);
    setPreviewError(null);
    setSelectedLanguage('__all__');
  }, [selected]);

  const filteredVoices = useMemo(() => {
    let result = voices;
    if (searchQuery) {
      result = result.filter((v) => voiceMatches(v.voice, searchQuery));
    }
    if (selectedLanguage && selectedLanguage !== '__all__') {
      result = result.filter((v) =>
        v.voice.languages?.some((l) => l === selectedLanguage),
      );
    }
    return result;
  }, [voices, searchQuery, selectedLanguage]);

  const stopPlayback = useCallback(() => {
    if (audioRef.current) {
      audioRef.current.pause();
      audioRef.current = null;
    }
    setPreviewingVoice(null);
  }, []);

  useEffect(() => {
    if (!open) stopPlayback();
  }, [open, stopPlayback]);

  useEffect(() => {
    return () => {
      if (audioRef.current) {
        audioRef.current.pause();
        audioRef.current = null;
      }
    };
  }, []);

  const handlePreview = useCallback(
    async (entry: VoiceEntry) => {
      if (previewingVoice === entry.key) {
        stopPlayback();
        return;
      }

      stopPlayback();
      setPreviewingVoice(entry.key);
      setPreviewError(null);

      try {
        const result = await homeClient.generateSpeech({
          input: SAMPLE_TEXT,
          provider: entry.providerId,
          model: entry.modelId,
          voice: entry.voice.id,
        });

        const blob = new Blob([result.audio], { type: result.contentType });
        const url = URL.createObjectURL(blob);
        const audio = new Audio(url);
        audioRef.current = audio;

        audio.onended = () => {
          URL.revokeObjectURL(url);
          setPreviewingVoice(null);
        };
        audio.onerror = () => {
          URL.revokeObjectURL(url);
          setPreviewingVoice(null);
          setPreviewError('Playback failed');
        };

        await audio.play();
      } catch (err) {
        setPreviewingVoice(null);
        setPreviewError(err instanceof Error ? err.message : 'Failed to generate speech');
      }
    },
    [previewingVoice, stopPlayback, homeClient],
  );

  if (!open) return null;

  const hasModels = modelOptions.length > 0;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={onClose}>
      <div
        className="w-full max-w-3xl max-h-[85vh] rounded-xl border border-border bg-card shadow-xl flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-border/60">
          <div className="flex items-center gap-3 min-w-0">
            <Volume2 className="h-5 w-5 text-muted-foreground shrink-0" />
            <div className="flex items-center gap-2 min-w-0">
              <h2 className="text-base font-semibold text-foreground shrink-0">Preview Voices</h2>
              {selected && (
                <div className="flex items-center gap-1.5 min-w-0">
                  <span className="text-muted-foreground">—</span>
                  <span className="text-sm text-muted-foreground truncate">{selected.providerLabel}/{selected.modelId}</span>
                  <button
                    type="button"
                    onClick={() => {
                      navigator.clipboard.writeText(selected.modelId);
                      setCopied(true);
                      setTimeout(() => setCopied(false), 1500);
                    }}
                    className="shrink-0 text-muted-foreground/50 hover:text-foreground transition"
                    title="Copy model ID"
                  >
                    {copied ? <Check className="h-3.5 w-3.5 text-green-500" /> : <Copy className="h-3.5 w-3.5" />}
                  </button>
                </div>
              )}
            </div>
          </div>
          <button type="button" onClick={onClose} className="text-muted-foreground hover:text-foreground transition">
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Filters */}
        {hasModels && (
          <div className="px-6 pt-4 pb-2 flex flex-wrap items-center gap-3">
            {/* Model selector */}
            <Select value={resolvedModelKey} onValueChange={setModelKey}>
              <SelectTrigger className="w-[240px]">
                <SelectPrimitive.Value placeholder="Select model..." />
              </SelectTrigger>
              <SelectContent>
                {modelOptions.map((opt) => (
                  <SelectItem key={`${opt.providerId}:${opt.modelId}`} value={`${opt.providerId}:${opt.modelId}`}>
                    {opt.providerLabel} / {opt.modelLabel} ({opt.voiceCount})
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            {/* Language filter */}
            {allLanguages.length > 1 && (
              <Select value={selectedLanguage} onValueChange={setSelectedLanguage}>
                <SelectTrigger className="w-[160px]">
                  <div className="flex items-center gap-1.5">
                    <Globe className="h-3.5 w-3.5 shrink-0 opacity-50" />
                    <SelectPrimitive.Value placeholder="All languages" />
                  </div>
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="__all__">All languages</SelectItem>
                  {allLanguages.map((lang) => (
                    <SelectItem key={lang} value={lang}>{lang}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}

            {/* Search */}
            <div className="relative flex-1 min-w-[140px]">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground/50" />
              <input
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="Search voices..."
                className="w-full rounded-md border border-input bg-transparent px-3 py-2 pl-9 text-sm shadow-sm ring-offset-background placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
              />
            </div>
          </div>
        )}

        {/* Body */}
        <div className="flex-1 overflow-y-auto px-6 py-4">
          {!hasModels ? (
            <div className="flex flex-col items-center justify-center py-12 text-center">
              <Volume2 className="h-8 w-8 text-muted-foreground/30 mb-3" />
              <p className="text-sm text-muted-foreground mb-4">No TTS voices available.</p>
              {onOpenProviders && (
                <button
                  type="button"
                  onClick={() => { onClose(); onOpenProviders(); }}
                  className="text-sm text-primary hover:text-primary/80 transition"
                >
                  Configure a TTS provider
                </button>
              )}
            </div>
          ) : filteredVoices.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-12 text-center">
              <Search className="h-6 w-6 text-muted-foreground/30 mb-3" />
              <p className="text-sm text-muted-foreground">
                No voices match &ldquo;{searchQuery}&rdquo;
                {selectedLanguage && selectedLanguage !== '__all__' && <span> in {selectedLanguage}</span>}
              </p>
            </div>
          ) : (
            <div className="space-y-2">
              {filteredVoices.map((entry) => {
                const isPlaying = previewingVoice === entry.key;
                const isOtherPlaying = previewingVoice !== null && !isPlaying;
                return (
                  <button
                    key={entry.voice.id}
                    type="button"
                    onClick={() => handlePreview(entry)}
                    disabled={isOtherPlaying}
                    className={cn(
                      'w-full flex items-center gap-4 rounded-lg border p-3 text-left transition',
                      isPlaying
                        ? 'border-primary/50 bg-primary/5'
                        : isOtherPlaying
                          ? 'opacity-40 cursor-not-allowed border-border/40'
                          : 'border-border/50 hover:border-border hover:bg-muted/20',
                    )}
                  >
                    <div className="shrink-0 w-8 h-8 rounded-full bg-muted/30 flex items-center justify-center">
                      {isPlaying ? (
                        <Loader2 className="h-4 w-4 text-primary animate-spin" />
                      ) : (
                        <Volume2 className="h-4 w-4 text-muted-foreground" />
                      )}
                    </div>

                    <div className="flex-1 grid grid-cols-2 gap-x-4 min-w-0">
                      <div className="min-w-0">
                        <span className="text-sm font-medium text-foreground">
                          {entry.voice.name}
                        </span>
                        {entry.voice.description && (
                          <p className="text-[11px] text-muted-foreground/60 truncate leading-tight mt-0.5">
                            {entry.voice.description}
                          </p>
                        )}
                      </div>
                      {entry.voice.languages && entry.voice.languages.length > 0 && (
                        <div className="flex items-start gap-1 min-w-0">
                          <Globe className="h-3 w-3 text-muted-foreground/40 mt-0.5 shrink-0" />
                          <span className="text-[11px] text-muted-foreground/50 truncate leading-tight">
                            {entry.voice.languages.join(', ')}
                          </span>
                        </div>
                      )}
                    </div>
                  </button>
                );
              })}
            </div>
          )}
        </div>

        {previewError && (
          <div className="px-6 py-2 border-t border-destructive/30 bg-destructive/5">
            <p className="text-xs text-destructive">{previewError}</p>
          </div>
        )}
      </div>
    </div>
  );
}
