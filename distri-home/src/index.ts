// Client exports
export { DistriHomeClient } from './DistriHomeClient';
export type { HomeStats, HomeStatsThread, ApiKey, Secret, PromptTemplate } from './DistriHomeClient';

// Provider exports
export { DistriHomeProvider, useDistriHome, useDistriHomeConfig, useDistriHomeNavigate, useDistriHomeClient } from './DistriHomeProvider';
export type { DistriHomeConfig, DistriHomeProviderProps, NavigateFunction } from './DistriHomeProvider';

// Hook exports
export { useHomeStats } from './hooks/useHomeStats';
export type { UseHomeStatsResult } from './hooks/useHomeStats';
export { useApiKeys } from './hooks/useApiKeys';
export type { UseApiKeysResult } from './hooks/useApiKeys';

// Component exports
export { Home } from './components/Home';
export { AgentDetails } from './components/AgentDetails';
export { ThreadsView } from './components/ThreadsView';
export { SettingsView } from './components/SettingsView';
export { SecretsView } from './components/SecretsView';
export type { HomeProps } from './components/Home';
export type { AgentDetailsProps } from './components/AgentDetails';
export type { ThreadsViewProps } from './components/ThreadsView';
export type { SettingsViewProps } from './components/SettingsView';
export type { SecretsViewProps } from './components/SecretsView';
export { PromptTemplatesView } from './components/PromptTemplatesView';
