import { useCallback, useEffect, useState } from 'react';
import { useDistriHome } from '../DistriHomeProvider';
import { AgentValidationResult, ValidationWarning } from '../DistriHomeClient';

export interface UseAgentValidationOptions {
  agentId: string;
  enabled?: boolean;
}

export interface UseAgentValidationResult {
  validation: AgentValidationResult | null;
  warnings: ValidationWarning[];
  loading: boolean;
  error: string | null;
  refetch: () => Promise<void>;
}

export function useAgentValidation({
  agentId,
  enabled = true,
}: UseAgentValidationOptions): UseAgentValidationResult {
  const { homeClient, isLoading: clientLoading } = useDistriHome();
  const [validation, setValidation] = useState<AgentValidationResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!homeClient || !agentId || !enabled) {
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      const data = await homeClient.validateAgent(agentId);
      setValidation(data);
      setError(null);
    } catch (err: unknown) {
      setValidation(null);
      // Silently handle validation errors - they shouldn't block the UI
      console.warn('[useAgentValidation] Failed to validate agent:', err);
      setError(err instanceof Error ? err.message : 'Failed to validate agent');
    } finally {
      setLoading(false);
    }
  }, [homeClient, agentId, enabled]);

  useEffect(() => {
    if (clientLoading) {
      return;
    }
    void load();
  }, [load, clientLoading]);

  return {
    validation,
    warnings: validation?.warnings ?? [],
    loading: loading || clientLoading,
    error,
    refetch: load,
  };
}
