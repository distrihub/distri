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
 * Home stats response from the server
 */
export interface HomeStats {
  total_agents?: number;
  total_owned_agents?: number;
  total_accessible_agents?: number;
  total_threads?: number;
  total_messages?: number;
  avg_time_per_run_ms?: number;
  latest_threads?: HomeStatsThread[];
  most_active_agent?: {
    id: string;
    name: string;
    count: number;
  };
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
}
