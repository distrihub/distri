import React, { createContext, useContext, useState, useEffect, ReactNode, useCallback, useMemo } from 'react';

interface ThreadContextType {
  // Get the last thread ID for an agent
  getLastThreadId: (agentId: string) => string | null;
  // Set the last thread ID for an agent
  setLastThreadId: (agentId: string, threadId: string) => void;
}

const ThreadContext = createContext<ThreadContextType | undefined>(undefined);

interface ThreadProviderProps {
  children: ReactNode;
}

export const ThreadProvider: React.FC<ThreadProviderProps> = ({ children }) => {
  const [threadStorage, setThreadStorage] = useState<Record<string, string>>({});

  // Load thread storage from localStorage on mount
  useEffect(() => {
    try {
      const stored = localStorage.getItem('thread-storage');
      if (stored) {
        const parsed = JSON.parse(stored);
        setThreadStorage(parsed);
      }
    } catch (error) {
      console.error('Error loading thread storage:', error);
    }
  }, []);

  // Save thread storage to localStorage whenever it changes
  useEffect(() => {
    try {
      localStorage.setItem('thread-storage', JSON.stringify(threadStorage));
    } catch (error) {
      console.error('Error saving thread storage:', error);
    }
  }, [threadStorage]);

  const getLastThreadId = useCallback((agentId: string): string | null => {
    return threadStorage[agentId] || null;
  }, [threadStorage]);

  const setLastThreadId = useCallback((agentId: string, threadId: string) => {
    setThreadStorage(prev => ({
      ...prev,
      [agentId]: threadId
    }));
  }, []);

  const value: ThreadContextType = useMemo(() => ({
    getLastThreadId,
    setLastThreadId,
  }), [getLastThreadId, setLastThreadId]);

  return (
    <ThreadContext.Provider value={value}>
      {children}
    </ThreadContext.Provider>
  );
};

export const useThreadContext = (): ThreadContextType => {
  const context = useContext(ThreadContext);
  if (!context) {
    throw new Error('useThreadContext must be used within a ThreadProvider');
  }
  return context;
}; 