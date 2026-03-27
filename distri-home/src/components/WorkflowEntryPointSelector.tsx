import { useState, useMemo, type ReactNode } from 'react';
import type { WorkflowDefinition, EntryPoint, WorkflowStep, StepStatus } from '@distri/core';
import { workflowProgress, countSteps, applyEntryPoint } from '@distri/core';
import {
  ChevronRight,
  Play,
  SkipForward,
  Circle,
  CheckCircle2,
  XCircle,
  Clock,
  Ban,
  FastForward,
  Info,
} from 'lucide-react';

// ── Types ─────────────────────────────────────────────────────────────────

export interface WorkflowEntryPointSelectorProps {
  /** The workflow definition with entry_points defined */
  workflow: WorkflowDefinition;
  /** Called when the user selects an entry point and clicks "Start" */
  onStart: (entryPointId: string | null, workflow: WorkflowDefinition) => void;
  /** Whether a workflow is currently running */
  isRunning?: boolean;
  /** Optional class name */
  className?: string;
}

// ── Step status icon ──────────────────────────────────────────────────────

function StepStatusIcon({ status }: { status: StepStatus }) {
  switch (status) {
    case 'done': return <CheckCircle2 className="h-3.5 w-3.5 sm:h-4 sm:w-4 text-green-500" />;
    case 'failed': return <XCircle className="h-3.5 w-3.5 sm:h-4 sm:w-4 text-red-500" />;
    case 'running': return <Clock className="h-3.5 w-3.5 sm:h-4 sm:w-4 text-blue-500 animate-spin" />;
    case 'skipped': return <FastForward className="h-3.5 w-3.5 sm:h-4 sm:w-4 text-muted-foreground" />;
    case 'blocked': return <Ban className="h-3.5 w-3.5 sm:h-4 sm:w-4 text-orange-500" />;
    case 'pending': return <Circle className="h-3.5 w-3.5 sm:h-4 sm:w-4 text-muted-foreground" />;
  }
}

// ── Entry point card ──────────────────────────────────────────────────────

function EntryPointCard({
  entryPoint,
  workflow,
  selected,
  onSelect,
}: {
  entryPoint: EntryPoint | null; // null = "from beginning"
  workflow: WorkflowDefinition;
  selected: boolean;
  onSelect: () => void;
}) {
  const preview = useMemo(() => {
    if (!entryPoint) return workflow;
    try {
      return applyEntryPoint(workflow, entryPoint.id);
    } catch {
      return workflow;
    }
  }, [entryPoint, workflow]);

  const counts = useMemo(() => countSteps(preview), [preview]);
  const stepsToRun = preview.steps.filter(s => s.status !== 'skipped').length;
  const stepsSkipped = counts.skipped;

  return (
    <button
      onClick={onSelect}
      className={`
        w-full text-left p-3 sm:p-4 rounded-lg border transition-colors
        ${selected
          ? 'border-primary bg-primary/5 ring-1 ring-primary/20'
          : 'border-border hover:border-muted-foreground/30 hover:bg-muted/50'
        }
      `}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <h4 className="text-sm sm:text-base font-medium text-foreground truncate">
            {entryPoint?.label ?? 'Start from beginning'}
          </h4>
          {entryPoint?.description && (
            <p className="text-xs sm:text-sm text-muted-foreground mt-0.5 line-clamp-2">
              {entryPoint.description}
            </p>
          )}
          <div className="flex items-center gap-3 mt-2 text-xs text-muted-foreground">
            <span className="flex items-center gap-1">
              <Play className="h-3 w-3" />
              {stepsToRun} step{stepsToRun !== 1 ? 's' : ''}
            </span>
            {stepsSkipped > 0 && (
              <span className="flex items-center gap-1">
                <SkipForward className="h-3 w-3" />
                {stepsSkipped} skipped
              </span>
            )}
            {entryPoint?.starts_at && (
              <span className="flex items-center gap-1">
                <ChevronRight className="h-3 w-3" />
                starts at <code className="bg-muted px-1 rounded text-[10px]">{entryPoint.starts_at}</code>
              </span>
            )}
          </div>
        </div>
        <div className={`
          h-4 w-4 rounded-full border-2 flex-shrink-0 mt-0.5 transition-colors
          ${selected ? 'border-primary bg-primary' : 'border-muted-foreground/30'}
        `}>
          {selected && (
            <svg className="h-full w-full text-primary-foreground" viewBox="0 0 16 16">
              <circle cx="8" cy="8" r="3" fill="currentColor" />
            </svg>
          )}
        </div>
      </div>
    </button>
  );
}

// ── Step preview ──────────────────────────────────────────────────────────

function StepPreview({ steps }: { steps: WorkflowStep[] }) {
  return (
    <div className="space-y-1">
      {steps.map((step) => (
        <div
          key={step.id}
          className={`
            flex items-center gap-2 py-1.5 px-2 rounded text-xs sm:text-sm
            ${step.status === 'skipped' ? 'opacity-40' : ''}
          `}
        >
          <StepStatusIcon status={step.status ?? 'pending'} />
          <span className="truncate flex-1">{step.label}</span>
          <span className="text-[10px] text-muted-foreground font-mono">{step.id}</span>
        </div>
      ))}
    </div>
  );
}

// ── Main component ────────────────────────────────────────────────────────

export function WorkflowEntryPointSelector({
  workflow,
  onStart,
  isRunning = false,
  className = '',
}: WorkflowEntryPointSelectorProps) {
  const entryPoints = workflow.entry_points ?? [];
  const hasEntryPoints = entryPoints.length > 0;

  // null = "from beginning"
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const selectedPreview = useMemo(() => {
    if (!selectedId) return workflow;
    try {
      return applyEntryPoint(workflow, selectedId);
    } catch {
      return workflow;
    }
  }, [selectedId, workflow]);

  const handleStart = () => {
    onStart(selectedId, selectedPreview);
  };

  if (!hasEntryPoints) {
    // No entry points — just show a simple start button
    return (
      <div className={`space-y-3 ${className}`}>
        <StepPreview steps={workflow.steps} />
        <button
          onClick={() => onStart(null, workflow)}
          disabled={isRunning}
          className="w-full flex items-center justify-center gap-2 px-4 py-2 bg-primary text-primary-foreground rounded-lg text-sm font-medium hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          {isRunning ? (
            <Clock className="h-4 w-4 animate-spin" />
          ) : (
            <Play className="h-4 w-4" />
          )}
          {isRunning ? 'Running...' : 'Start Workflow'}
        </button>
      </div>
    );
  }

  return (
    <div className={`space-y-4 ${className}`}>
      {/* Entry point selector */}
      <div>
        <h3 className="text-sm font-medium text-foreground mb-2">Choose entry point</h3>
        <div className="space-y-2">
          {/* Default: from beginning */}
          <EntryPointCard
            entryPoint={null}
            workflow={workflow}
            selected={selectedId === null}
            onSelect={() => setSelectedId(null)}
          />
          {/* Named entry points */}
          {entryPoints.map((ep) => (
            <EntryPointCard
              key={ep.id}
              entryPoint={ep}
              workflow={workflow}
              selected={selectedId === ep.id}
              onSelect={() => setSelectedId(ep.id)}
            />
          ))}
        </div>
      </div>

      {/* Required inputs hint */}
      {selectedId && (() => {
        const ep = entryPoints.find(e => e.id === selectedId);
        if (!ep?.required_inputs?.length) return null;
        return (
          <div className="flex items-start gap-2 p-2.5 bg-muted/50 rounded-lg text-xs text-muted-foreground">
            <Info className="h-3.5 w-3.5 flex-shrink-0 mt-0.5" />
            <span>
              Required inputs: {ep.required_inputs.map(r => (
                <code key={r} className="bg-muted px-1 rounded mx-0.5">{r}</code>
              ))}
            </span>
          </div>
        );
      })()}

      {/* Step preview */}
      <div>
        <h3 className="text-sm font-medium text-foreground mb-2">Steps</h3>
        <div className="border rounded-lg p-2 bg-muted/20 max-h-60 overflow-y-auto">
          <StepPreview steps={selectedPreview.steps} />
        </div>
      </div>

      {/* Start button */}
      <button
        onClick={handleStart}
        disabled={isRunning}
        className="w-full flex items-center justify-center gap-2 px-4 py-2.5 bg-primary text-primary-foreground rounded-lg text-sm font-medium hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
      >
        {isRunning ? (
          <Clock className="h-4 w-4 animate-spin" />
        ) : (
          <Play className="h-4 w-4" />
        )}
        {isRunning ? 'Running...' : selectedId ? `Start from "${entryPoints.find(e => e.id === selectedId)?.label}"` : 'Start from beginning'}
      </button>
    </div>
  );
}
