/**
 * Mock implementation of @distri/react for testing
 */

export const useDistri = () => ({
  client: null,
  isLoading: false,
  error: null,
});

export const useAgent = () => ({
  agent: null,
  isLoading: false,
  error: null,
});

export const useThreads = () => ({
  threads: [],
  isLoading: false,
  error: null,
  hasMore: false,
  loadMore: () => {},
});

export const useAgentsByUsage = () => ({
  agents: [],
  isLoading: false,
  error: null,
});

export const DistriProvider = ({ children }: { children: React.ReactNode }) => children;
