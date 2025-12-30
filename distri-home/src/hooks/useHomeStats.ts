import { useCallback, useEffect, useState } from 'react';
import { useDistriHome } from '../DistriHomeProvider';
import { HomeStats } from '../DistriHomeClient';

export interface UseHomeStatsResult {
  stats: HomeStats | null;
  loading: boolean;
  error: string | null;
  refetch: () => Promise<void>;
}

export function useHomeStats(): UseHomeStatsResult {
  const { homeClient, isLoading: clientLoading } = useDistriHome();
  const [stats, setStats] = useState<HomeStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!homeClient) {
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      const data = await homeClient.getHomeStats();
      setStats(data);
      setError(null);
    } catch (err: unknown) {
      setStats(null);
      setError(err instanceof Error ? err.message : 'Failed to load stats');
    } finally {
      setLoading(false);
    }
  }, [homeClient]);

  useEffect(() => {
    if (clientLoading) {
      return;
    }
    void load();
  }, [load, clientLoading]);

  return { stats, loading: loading || clientLoading, error, refetch: load };
}
