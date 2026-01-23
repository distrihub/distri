import { createContext, useContext, useMemo, ReactNode } from 'react';
import { DistriClient, DistriClientConfig } from '@distri/core';
import { DistriHomeClient } from './DistriHomeClient';

export interface HomeWidget {
  id: string;
  render: () => ReactNode;
}

export interface DistriHomeConfig {

  /**
   * Custom navigation path generators.
   * Use this to customize internal links (e.g. agent details).
   */
  navigationPaths?: {
    /**
     * Generate path for agent details page
     * @default "agents/<id>"
     */
    agentDetails?: (agentId: string) => string;
  };
  /**
   * Custom tabs to add to the Agent Details page.
   */
  customTabs?: {
    id: string;
    label: string;
    icon?: ReactNode;
    render: (props: { agentId: string }) => ReactNode;
  }[];
  /**
   * Custom widgets to add to the Home page overview row.
   * These render as additional cards in the top stats section.
   */
  homeWidgets?: HomeWidget[];
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
  workspaceId: string | undefined;
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

const DEFAULT_CONFIG: DistriHomeConfig = {};

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

  // Track workspaceId for triggering refetches when it changes
  const workspaceId = distriClient?.workspaceId;

  return (
    <DistriHomeContext.Provider
      value={{
        config: mergedConfig,
        navigate: onNavigate,
        homeClient,
        distriClient,
        isLoading: false,
        workspaceId,
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
