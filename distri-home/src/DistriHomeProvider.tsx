import { createContext, useContext, useMemo, ReactNode } from 'react';
import { DistriClient, DistriClientConfig } from '@distri/core';
import { DistriHomeClient } from './DistriHomeClient';

export interface DistriHomeConfig {
  /**
   * Enable API Keys management in SettingsView.
   * Set to false for local/OSS versions.
   * @default true
   */
  enableApiKeys?: boolean;
  /**
   * Enable Account & billing section in SettingsView.
   * Set to false for local/OSS versions.
   * @default true
   */
  enableAccountBilling?: boolean;
}

export interface NavigateFunction {
  (path: string): void;
}

interface DistriHomeContextValue {
  config: DistriHomeConfig;
  navigate: NavigateFunction;
  homeClient: DistriHomeClient | null;
  distriClient: DistriClient | null;
  isLoading: boolean;
}

const DistriHomeContext = createContext<DistriHomeContextValue | null>(null);

export interface DistriHomeProviderProps {
  /**
   * Either provide a DistriClient instance or config to create one
   */
  client?: DistriClient;
  clientConfig?: DistriClientConfig;
  config?: DistriHomeConfig;
  /**
   * Navigation callback - called when components need to navigate.
   * Integrate with your router (e.g., useNavigate from react-router-dom)
   */
  onNavigate: NavigateFunction;
  children: ReactNode;
}

const DEFAULT_CONFIG: DistriHomeConfig = {
  enableApiKeys: true,
  enableAccountBilling: true,
};

export function DistriHomeProvider({
  client,
  clientConfig,
  config = {},
  onNavigate,
  children,
}: DistriHomeProviderProps) {
  const mergedConfig = { ...DEFAULT_CONFIG, ...config };

  const { homeClient, distriClient } = useMemo(() => {
    if (client) {
      return {
        homeClient: new DistriHomeClient(client),
        distriClient: client,
      };
    }
    if (clientConfig) {
      const newClient = new DistriClient(clientConfig);
      return {
        homeClient: new DistriHomeClient(newClient),
        distriClient: newClient,
      };
    }
    return { homeClient: null, distriClient: null };
  }, [client, clientConfig]);

  return (
    <DistriHomeContext.Provider
      value={{
        config: mergedConfig,
        navigate: onNavigate,
        homeClient,
        distriClient,
        isLoading: false,
      }}
    >
      {children}
    </DistriHomeContext.Provider>
  );
}

export function useDistriHome(): DistriHomeContextValue {
  const context = useContext(DistriHomeContext);
  if (!context) {
    throw new Error('useDistriHome must be used within a DistriHomeProvider');
  }
  return context;
}

export function useDistriHomeConfig(): DistriHomeConfig {
  const { config } = useDistriHome();
  return config;
}

export function useDistriHomeNavigate(): NavigateFunction {
  const { navigate } = useDistriHome();
  return navigate;
}

export function useDistriHomeClient(): DistriHomeClient | null {
  const { homeClient } = useDistriHome();
  return homeClient;
}
