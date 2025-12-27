import { useCallback, useEffect, useState } from 'react';
import { useDistriHome } from '../DistriHomeProvider';
import { ApiKey } from '../DistriHomeClient';

export interface UseApiKeysResult {
  keys: ApiKey[];
  loading: boolean;
  error: string | null;
  refetch: () => Promise<void>;
  createKey: (label: string) => Promise<ApiKey>;
  revokeKey: (keyId: string) => Promise<void>;
}

export function useApiKeys(): UseApiKeysResult {
  const { homeClient, config } = useDistriHome();
  const [keys, setKeys] = useState<ApiKey[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!homeClient || !config.enableApiKeys) {
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      const data = await homeClient.listApiKeys();
      setKeys(data);
      setError(null);
    } catch (err: unknown) {
      setKeys([]);
      setError(err instanceof Error ? err.message : 'Failed to load API keys');
    } finally {
      setLoading(false);
    }
  }, [homeClient, config.enableApiKeys]);

  useEffect(() => {
    void load();
  }, [load]);

  const createKey = useCallback(
    async (label: string): Promise<ApiKey> => {
      if (!homeClient) {
        throw new Error('Client not initialized');
      }
      const key = await homeClient.createApiKey(label);
      await load();
      return key;
    },
    [homeClient, load]
  );

  const revokeKey = useCallback(
    async (keyId: string): Promise<void> => {
      if (!homeClient) {
        throw new Error('Client not initialized');
      }
      await homeClient.revokeApiKey(keyId);
      await load();
    },
    [homeClient, load]
  );

  return { keys, loading, error, refetch: load, createKey, revokeKey };
}
