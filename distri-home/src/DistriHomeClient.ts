import { DistriClient, DistriClientConfig } from '@distri/core';

/**
 * Thread returned in home stats
 */
export interface HomeStatsThread {
  id: string;
  title: string;
  agent_id: string;
  agent_name: string;
  updated_at: string;
  message_count: number;
  last_message?: string | null;
}

/**
 * Recently used agent info
 */
export interface RecentlyUsedAgent {
  id: string;
  name: string;
  description?: string | null;
  last_used_at: string;
}

/**
 * Agent usage info - agents sorted by thread count
 */
export interface AgentUsageInfo {
  agent_id: string;
  agent_name: string;
  thread_count: number;
}

/**
 * Custom metric for dynamic stats display
 */
export interface CustomMetric {
  label: string;
  value: string;
  helper?: string;
  limit?: string;
  raw_value?: number;
  raw_limit?: number;
}

/**
 * Home stats response from the server
 */
export interface HomeStats {
  total_agents?: number;
  total_owned_agents?: number;
  total_accessible_agents?: number;
  total_threads?: number;
  total_messages?: number;
  avg_run_time_ms?: number;
  latest_threads?: HomeStatsThread[];
  most_active_agent?: {
    id: string;
    name: string;
    count: number;
  };
  recently_used_agents?: RecentlyUsedAgent[];
  custom_metrics?: Record<string, CustomMetric>;
}

/**
 * API Key type
 */
export interface ApiKey {
  id: string;
  label?: string;
  name?: string;
  key?: string;
  created_at?: string;
}

/**
 * DistriHomeClient extends DistriClient with home-specific methods.
 * Uses DistriClient's fetch method for authenticated requests.
 */
export class DistriHomeClient {
  private client: DistriClient;

  constructor(clientOrConfig: DistriClient | DistriClientConfig) {
    if (clientOrConfig instanceof DistriClient) {
      this.client = clientOrConfig;
    } else {
      this.client = new DistriClient(clientOrConfig);
    }
  }

  /**
   * Get the underlying DistriClient
   */
  get distriClient(): DistriClient {
    return this.client;
  }

  /**
   * Get the base URL
   */
  get baseUrl(): string {
    return this.client.baseUrl;
  }

  /**
   * Get home stats from Distri server
   * Uses DistriClient's fetch for authentication
   */
  async getHomeStats(): Promise<HomeStats> {
    const response = await this.client.fetch('/home/stats');

    if (!response.ok) {
      throw new Error(`Failed to fetch home stats: ${response.statusText}`);
    }

    return await response.json();
  }

  /**
   * Get agents sorted by usage (thread count)
   */
  async getAgentsByUsage(): Promise<AgentUsageInfo[]> {
    const response = await this.client.fetch('/agents/usage');

    if (!response.ok) {
      throw new Error(`Failed to fetch agents by usage: ${response.statusText}`);
    }

    return await response.json();
  }

  /**
   * List API keys
   */
  async listApiKeys(): Promise<ApiKey[]> {
    const response = await this.client.fetch('/api-keys');

    if (!response.ok) {
      throw new Error(`Failed to fetch API keys: ${response.statusText}`);
    }

    return await response.json();
  }

  /**
   * Create a new API key
   */
  async createApiKey(label: string): Promise<ApiKey> {
    const response = await this.client.fetch('/api-keys', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ label }),
    });

    if (!response.ok) {
      throw new Error(`Failed to create API key: ${response.statusText}`);
    }

    return await response.json();
  }

  /**
   * Revoke an API key
   */
  async revokeApiKey(keyId: string): Promise<void> {
    const response = await this.client.fetch(`/api-keys/${keyId}`, {
      method: 'DELETE',
    });

    if (!response.ok) {
      throw new Error(`Failed to revoke API key: ${response.statusText}`);
    }
  }

  // ---- Secrets ----

  /**
   * List all secrets
   */
  async listSecrets(): Promise<Secret[]> {
    const response = await this.client.fetch('/secrets');

    if (!response.ok) {
      throw new Error(`Failed to fetch secrets: ${response.statusText}`);
    }

    return await response.json();
  }

  /**
   * Create or update a secret
   */
  async createSecret(key: string, value: string): Promise<Secret> {
    const response = await this.client.fetch('/secrets', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ key, value }),
    });

    if (!response.ok) {
      throw new Error(`Failed to create secret: ${response.statusText}`);
    }

    return await response.json();
  }

  /**
   * Delete a secret
   */
  async deleteSecret(id: string): Promise<void> {
    const response = await this.client.fetch(`/secrets/${id}`, {
      method: 'DELETE',
    });

    if (!response.ok) {
      throw new Error(`Failed to delete secret: ${response.statusText}`);
    }
  }

  /**
   * List provider secret definitions
   * Returns the list of supported providers and their required secret keys
   */
  async listProviderDefinitions(): Promise<ProviderSecretDefinition[]> {
    const response = await this.client.fetch('/secrets/providers');

    if (!response.ok) {
      throw new Error(`Failed to fetch provider definitions: ${response.statusText}`);
    }

    return await response.json();
  }

  /**
   * Validate an agent's configuration
   * Returns validation results including any warnings (e.g., missing secrets)
   */
  async validateAgent(agentId: string): Promise<AgentValidationResult> {
    const response = await this.client.fetch(`/agents/${agentId}/validate`);

    if (!response.ok) {
      throw new Error(`Failed to validate agent: ${response.statusText}`);
    }

    return await response.json();
  }

  // ---- Prompt Templates ----

  /**
   * List all prompt templates (system + user)
   */
  async listPromptTemplates(): Promise<PromptTemplate[]> {
    const response = await this.client.fetch('/prompt-templates');

    if (!response.ok) {
      throw new Error(`Failed to fetch prompt templates: ${response.statusText}`);
    }

    return await response.json();
  }

  /**
   * Create a new prompt template
   */
  async createPromptTemplate(name: string, template: string): Promise<PromptTemplate> {
    const response = await this.client.fetch('/prompt-templates', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ name, template }),
    });

    if (!response.ok) {
      throw new Error(`Failed to create prompt template: ${response.statusText}`);
    }

    return await response.json();
  }

  /**
   * Update a prompt template
   */
  async updatePromptTemplate(id: string, name: string, template: string): Promise<PromptTemplate> {
    const response = await this.client.fetch(`/prompt-templates/${id}`, {
      method: 'PUT',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ name, template }),
    });

    if (!response.ok) {
      throw new Error(`Failed to update prompt template: ${response.statusText}`);
    }

    return await response.json();
  }

  /**
   * Delete a prompt template
   */
  async deletePromptTemplate(id: string): Promise<void> {
    const response = await this.client.fetch(`/prompt-templates/${id}`, {
      method: 'DELETE',
    });

    if (!response.ok) {
      throw new Error(`Failed to delete prompt template: ${response.statusText}`);
    }
  }

  /**
   * Clone a prompt template
   */
  async clonePromptTemplate(id: string): Promise<PromptTemplate> {
    const response = await this.client.fetch(`/prompt-templates/${id}/clone`, {
      method: 'POST',
    });

    if (!response.ok) {
      throw new Error(`Failed to clone prompt template: ${response.statusText}`);
    }

    return await response.json();
  }

  // ---- Sessions ----

  /**
   * List sessions
   */
  async listSessions(options?: {
    threadId?: string;
    limit?: number;
    offset?: number;
  }): Promise<SessionSummary[]> {
    const params = new URLSearchParams();
    if (options?.threadId) {
      params.append('thread_id', options.threadId);
    }
    if (options?.limit) {
      params.append('limit', options.limit.toString());
    }
    if (options?.offset) {
      params.append('offset', options.offset.toString());
    }

    const response = await this.client.fetch(`/sessions?${params.toString()}`);

    if (!response.ok) {
      throw new Error(`Failed to list sessions: ${response.statusText}`);
    }

    return await response.json();
  }

  /**
   * Get all values for a session
   */
  async getSessionValues(sessionId: string): Promise<Record<string, any>> {
    const response = await this.client.fetch(`/sessions/${sessionId}/values`);

    if (!response.ok) {
      throw new Error(`Failed to get session values: ${response.statusText}`);
    }

    const data = await response.json();
    return data.values;
  }
}

// Types for secrets and prompt templates
export interface Secret {
  id: string;
  key: string;
  masked_value: string;
  created_at?: string;
  updated_at?: string;
}

/**
 * Definition of a secret key for a provider
 */
export interface SecretKeyDefinition {
  key: string;
  label: string;
  placeholder: string;
  required?: boolean;
}

/**
 * Definition of a provider's secret requirements
 */
export interface ProviderSecretDefinition {
  id: string;
  label: string;
  keys: SecretKeyDefinition[];
}

export interface PromptTemplate {
  id: string;
  name: string;
  template: string;
  description?: string;
  version?: string;
  is_system?: boolean;
  created_at?: string;
  updated_at?: string;
}

export interface SessionSummary {
  session_id: string;
  keys: string[];
  key_count: number;
  updated_at?: string;
}

/**
 * Severity level for validation warnings
 */
export type ValidationWarningSeverity = 'warning' | 'error';

/**
 * A single validation warning
 */
export interface ValidationWarning {
  code: string;
  message: string;
  severity: ValidationWarningSeverity;
}

/**
 * Result from agent validation
 */
export interface AgentValidationResult {
  valid: boolean;
  warnings: ValidationWarning[];
}
