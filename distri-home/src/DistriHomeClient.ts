import { DistriClient, DistriClientConfig, TtsSpeechRequest, TtsSpeechResponse } from '@distri/core';

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
    thread_count: number;
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

export interface DetailedThreadListParams {
  agent_id?: string;
  external_id?: string;
  user_id?: string;
  channel_id?: string;
  bot_id?: string;
  search?: string;
  from_date?: string;
  to_date?: string;
  limit?: number;
  offset?: number;
}

export interface DetailedThread {
  id: string;
  title: string;
  agent_id: string;
  agent_name: string;
  updated_at: string;
  message_count: number;
  last_message?: string | null;
  user_id?: string | null;
  user_name?: string | null;
  user_email?: string | null;
  external_id?: string | null;
  channel_id?: string | null;
  channel_name?: string | null;
  channel_provider?: string | null;
  input_tokens?: number;
  output_tokens?: number;
  total_tokens?: number;
  tags?: string[];
}

export interface DetailedThreadsResponse {
  threads: DetailedThread[];
  total: number;
  page: number;
  page_size: number;
}

export interface UserChannelSummary {
  channel_id: string;
  provider: string;
  name?: string | null;
  chat_id?: string | null;
}

export interface UserListItem {
  id: string;
  name?: string | null;
  email?: string | null;
  created_at: string;
  updated_at: string;
  last_seen_at?: string | null;
  thread_count: number;
  message_count: number;
  channel_count: number;
  channels: UserChannelSummary[];
}

export interface UserListResponse {
  users: UserListItem[];
  total: number;
  page: number;
  page_size: number;
}

export interface UserListParams {
  search?: string;
  channel_id?: string;
  sort_by?: 'created_at' | 'updated_at' | 'message_count' | 'channel_count' | 'name';
  sort_order?: 'asc' | 'desc';
  limit?: number;
  offset?: number;
}

export interface UserDetail extends UserListItem {
  external_ids: string[];
}

export interface SendUserTestMessageRequest {
  channel_id: string;
  message: string;
}

export interface ChannelConversation {
  channel_id: string;
  chat_id: string;
  active: boolean;
  thread_id: string | null;
  created_at: string | null;
}

export interface ChannelDetail {
  id: string;
  provider: string;
  bot_username: string | null;
  auth_mode: string;
  agent_id: string;
  active: boolean;
  workspace_id: string;
  connection_id: string | null;
  conversation_count: number;
  conversations: ChannelConversation[];
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
   * Get agents sorted by usage (thread count).
   * Includes all registered agents, even those with 0 threads.
   * Optionally filter by name with search parameter.
   */
  async getAgentsByUsage(options?: { search?: string }): Promise<AgentUsageInfo[]> {
    const params = new URLSearchParams();
    if (options?.search) {
      params.set('search', options.search);
    }
    const query = params.toString();
    const url = query ? `/threads/agents?${query}` : '/threads/agents';
    const response = await this.client.fetch(url);

    if (!response.ok) {
      throw new Error(`Failed to fetch agents by usage: ${response.statusText}`);
    }

    return await response.json();
  }

  async listDetailedThreads(params: DetailedThreadListParams = {}): Promise<DetailedThreadsResponse> {
    const searchParams = new URLSearchParams();
    if (params.agent_id) searchParams.set('agent_id', params.agent_id);
    if (params.external_id) searchParams.set('external_id', params.external_id);
    if (params.user_id) searchParams.set('user_id', params.user_id);
    if (params.channel_id) searchParams.set('channel_id', params.channel_id);
    if (params.bot_id) searchParams.set('bot_id', params.bot_id);
    if (params.search) searchParams.set('search', params.search);
    if (params.from_date) searchParams.set('from_date', params.from_date);
    if (params.to_date) searchParams.set('to_date', params.to_date);
    if (params.limit !== undefined) searchParams.set('limit', params.limit.toString());
    if (params.offset !== undefined) searchParams.set('offset', params.offset.toString());

    const query = searchParams.toString();
    const url = query ? `/threads/detailed?${query}` : '/threads/detailed';
    const response = await this.client.fetch(url);

    if (response.ok) {
      return await response.json();
    }

    if (response.status !== 404) {
      throw new Error(`Failed to fetch detailed threads: ${response.statusText}`);
    }

    const fallback = await this.client.getThreads({
      agent_id: params.agent_id,
      external_id: params.external_id,
      search: params.search,
      from_date: params.from_date,
      to_date: params.to_date,
      limit: params.limit,
      offset: params.offset,
    });

    return {
      total: fallback.total,
      page: fallback.page,
      page_size: fallback.page_size,
      threads: fallback.threads.map((thread) => ({
        id: thread.id,
        title: thread.title,
        agent_id: thread.agent_id,
        agent_name: thread.agent_name,
        updated_at: thread.updated_at,
        message_count: thread.message_count,
        last_message: thread.last_message,
        user_id: thread.user_id ?? null,
        user_name: null,
        user_email: null,
        external_id: thread.external_id ?? null,
        channel_id: thread.channel_id ?? null,
        channel_name: thread.channel_name ?? null,
        channel_provider: null,
        input_tokens: thread.input_tokens,
        output_tokens: thread.output_tokens,
        total_tokens: thread.total_tokens,
        tags: thread.tags,
      })),
    };
  }

  async listUsers(params: UserListParams = {}): Promise<UserListResponse> {
    const searchParams = new URLSearchParams();
    if (params.search) searchParams.set('search', params.search);
    if (params.channel_id) searchParams.set('channel_id', params.channel_id);
    if (params.sort_by) searchParams.set('sort_by', params.sort_by);
    if (params.sort_order) searchParams.set('sort_order', params.sort_order);
    if (params.limit !== undefined) searchParams.set('limit', params.limit.toString());
    if (params.offset !== undefined) searchParams.set('offset', params.offset.toString());

    const query = searchParams.toString();
    const url = query ? `/users?${query}` : '/users';
    const response = await this.client.fetch(url);

    if (response.ok) {
      return await response.json();
    }
    if (response.status === 404) {
      return { users: [], total: 0, page: 1, page_size: params.limit ?? 30 };
    }
    throw new Error(`Failed to fetch users: ${response.statusText}`);
  }

  async getUser(userId: string): Promise<UserDetail> {
    const response = await this.client.fetch(`/users/${encodeURIComponent(userId)}`);
    if (response.ok) {
      return await response.json();
    }
    if (response.status === 404) {
      throw new Error('User not found');
    }
    throw new Error(`Failed to fetch user: ${response.statusText}`);
  }

  async deleteUser(userId: string): Promise<void> {
    const response = await this.client.fetch(`/users/${encodeURIComponent(userId)}`, {
      method: 'DELETE',
    });
    if (!response.ok) {
      throw new Error(`Failed to delete user: ${response.statusText}`);
    }
  }

  async getChannel(channelId: string): Promise<ChannelDetail> {
    const response = await this.client.fetch(`/channels/${encodeURIComponent(channelId)}`);
    if (response.ok) {
      return await response.json();
    }
    if (response.status === 404) {
      throw new Error('Channel not found');
    }
    throw new Error(`Failed to fetch channel: ${response.statusText}`);
  }

  async sendUserTestMessage(
    userId: string,
    payload: SendUserTestMessageRequest,
  ): Promise<void> {
    const response = await this.client.fetch(`/users/${encodeURIComponent(userId)}/test-message`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    if (!response.ok) {
      throw new Error(`Failed to send test message: ${response.statusText}`);
    }
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
   * Delete a secret by key name
   */
  async deleteSecret(key: string): Promise<void> {
    const response = await this.client.fetch(`/secrets/${encodeURIComponent(key)}`, {
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
   * List provider definitions with their keys and models.
   */
  async listProviders(): Promise<ModelProviderDefinition[]> {
    const response = await this.client.fetch('/providers');
    if (!response.ok) {
      throw new Error(`Failed to fetch providers: ${response.statusText}`);
    }
    return await response.json();
  }

  /**
   * List all models with denormalized provider info and configured status.
   */
  async listModels(): Promise<ModelWithProvider[]> {
    const response = await this.client.fetch('/models');
    if (!response.ok) {
      throw new Error(`Failed to fetch models: ${response.statusText}`);
    }
    return await response.json();
  }

  /**
   * List known provider types.
   */
  async listProviderTypes(): Promise<ProviderTypeInfo[]> {
    const response = await this.client.fetch('/provider-types');
    if (!response.ok) {
      throw new Error(`Failed to fetch provider types: ${response.statusText}`);
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

  /**
   * Clone an agent by name into the current workspace
   */
  async cloneAgent(agentName: string): Promise<void> {
    const response = await this.client.fetch(`/agents/${agentName}/clone`, {
      method: 'POST',
    });

    if (!response.ok) {
      throw new Error(`Failed to clone agent: ${response.statusText}`);
    }
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

  // ---- Skills ----

  /**
   * List all skills
   */
  async listSkills(): Promise<SkillRecord[]> {
    const response = await this.client.fetch('/skills');
    if (!response.ok) {
      throw new Error(`Failed to fetch skills: ${response.statusText}`);
    }
    return await response.json();
  }

  /**
   * Get a skill by ID
   */
  async getSkill(id: string): Promise<SkillRecord> {
    const response = await this.client.fetch(`/skills/${id}`);
    if (!response.ok) {
      throw new Error(`Failed to fetch skill: ${response.statusText}`);
    }
    return await response.json();
  }

  /**
   * Create a new skill
   */
  async createSkill(data: NewSkill): Promise<SkillRecord> {
    const response = await this.client.fetch('/skills', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(data),
    });
    if (!response.ok) {
      throw new Error(`Failed to create skill: ${response.statusText}`);
    }
    return await response.json();
  }

  /**
   * Update an existing skill
   */
  async updateSkill(id: string, data: UpdateSkill): Promise<SkillRecord> {
    const response = await this.client.fetch(`/skills/${id}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(data),
    });
    if (!response.ok) {
      throw new Error(`Failed to update skill: ${response.statusText}`);
    }
    return await response.json();
  }

  /**
   * Delete a skill
   */
  async deleteSkill(id: string): Promise<void> {
    const response = await this.client.fetch(`/skills/${id}`, {
      method: 'DELETE',
    });
    if (!response.ok) {
      throw new Error(`Failed to delete skill: ${response.statusText}`);
    }
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

  /**
   * List configured secrets with their set/unset status
   */
  async listConfiguredSecrets(): Promise<ConfiguredField[]> {
    const response = await this.client.fetch('/secrets/configured');
    if (!response.ok) throw new Error(`Failed: ${response.statusText}`);
    return await response.json();
  }

  // ---- Providers ----

  /**
   * Upsert a provider: saves secrets and optional custom config in a single call.
   * For built-in providers (openai, anthropic, etc.), pass just secrets.
   * For custom providers, also pass config with id, name, base_url.
   */
  async upsertProvider(request: UpsertProviderRequest): Promise<UpsertProviderResponse> {
    const response = await this.client.fetch('/providers', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(request),
    });
    if (!response.ok) {
      const text = await response.text();
      let msg = response.statusText;
      try { msg = JSON.parse(text)?.error || msg; } catch {}
      throw new Error(`Failed to save provider: ${msg}`);
    }
    return await response.json();
  }

  /**
   * Delete a provider and all its associated secrets and config.
   */
  async deleteProvider(providerId: string): Promise<void> {
    const response = await this.client.fetch(`/providers/${encodeURIComponent(providerId)}`, {
      method: 'DELETE',
    });
    if (!response.ok) {
      throw new Error(`Failed to delete provider: ${response.statusText}`);
    }
  }

  /**
   * Get the workspace's default model settings.
   */
  async getDefaultModelSettings(): Promise<any> {
    const response = await this.client.fetch('/providers/default-model');
    if (!response.ok) throw new Error(`Failed: ${response.statusText}`);
    return await response.json();
  }

  /**
   * Get workspace settings for the current workspace
   */
  async getWorkspaceSettings(): Promise<Record<string, any>> {
    const response = await this.client.fetch('/workspaces/current');
    if (!response.ok) {
      // Fallback: try to get settings from workspace list
      const listResp = await this.client.fetch('/workspaces');
      if (!listResp.ok) throw new Error(`Failed: ${listResp.statusText}`);
      const workspaces = await listResp.json();
      const ws = Array.isArray(workspaces) ? workspaces[0] : workspaces?.data?.[0];
      return ws?.settings ?? {};
    }
    const ws = await response.json();
    return ws?.settings ?? {};
  }

  // ---- TTS Models ----

  /**
   * List available TTS models from the server.
   */
  async listTtsModels(): Promise<{ models: Model[] }> {
    const response = await this.client.fetch('/audio/models');
    if (!response.ok) {
      throw new Error(`Failed to fetch TTS models: ${response.statusText}`);
    }
    return await response.json();
  }

  /**
   * List providers that have TTS models.
   */
  async listTtsProviders(): Promise<ModelProviderDefinition[]> {
    const response = await this.client.fetch('/audio/providers');
    if (!response.ok) {
      throw new Error(`Failed to fetch TTS providers: ${response.statusText}`);
    }
    return await response.json();
  }

  // ---- TTS Speech ----

  /**
   * Generate speech from text. Convenience wrapper around DistriClient.ttsSpeech().
   */
  async generateSpeech(request: TtsSpeechRequest): Promise<TtsSpeechResponse> {
    return this.client.ttsSpeech(request);
  }

  /**
   * Update workspace settings for the current workspace.
   * Uses /workspaces/current to resolve the workspace from the X-Workspace-Id header.
   */
  async updateWorkspaceSettings(settings: Record<string, any>): Promise<void> {
    // Get current workspace from header-based endpoint
    const currentResp = await this.client.fetch('/workspaces/current');
    if (!currentResp.ok) throw new Error(`Failed to get current workspace`);
    const ws = await currentResp.json();
    if (!ws?.id) throw new Error('No workspace found');

    const response = await this.client.fetch(`/workspaces/${ws.id}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ settings }),
    });
    if (!response.ok) throw new Error(`Failed: ${response.statusText}`);
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
  sensitive?: boolean;
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

// Skill types

export interface SkillRecord {
  id: string;
  name: string;
  description?: string;
  content: string;
  tags: string[];
  is_public: boolean;
  is_system: boolean;
  star_count?: number;
  clone_count?: number;
  created_at?: string;
  updated_at?: string;
}

export interface NewSkill {
  name: string;
  description?: string;
  content: string;
  tags?: string[];
  is_public?: boolean;
}

export interface UpdateSkill {
  name?: string;
  description?: string;
  content?: string;
  tags?: string[];
  is_public?: boolean;
}

export interface ConfiguredField {
  key: string;
  is_set: boolean;
  value?: string | null;
  sensitive: boolean;
}

export interface CustomProviderConfig {
  id: string;
  name: string;
  base_url: string;
  project_id?: string | null;
}

export interface CustomModelEntry {
  provider: string;
  model: string;
}

/**
 * Request to upsert a provider — saves secrets, config, models, and default in one call.
 */
export interface UpsertProviderRequest {
  provider_id: string;
  secrets?: Record<string, string>;
  config?: CustomProviderConfig;
  custom_models?: CustomModelEntry[];
  /** Set to a "provider/model" string, or empty string to clear */
  default_model?: string;
}

/**
 * Response from upserting a provider.
 */
export interface UpsertProviderResponse {
  provider_id: string;
  secrets_saved: number;
  config_saved: boolean;
}

// Model & Provider types

export interface TtsVoiceInfo {
  id: string;
  name: string;
  description?: string | null;
  languages?: string[];
}

export type ModelCapability = 'completion' | 'tts' | 'stt';

export type ModelPricing =
  | { type: 'completion'; input: number; output: number; cached_input?: number }
  | { type: 'tts'; per_1m_chars: number }
  | { type: 'stt'; per_minute: number };

export interface Model {
  id: string;
  name: string;
  capability: ModelCapability;
  context_window?: number;
  pricing?: ModelPricing;
  voices?: TtsVoiceInfo[];
  formats?: string[];
}

export interface ModelWithProvider extends Model {
  provider_id: string;
  provider_label: string;
  configured: boolean;
}

export interface ProviderTypeInfo {
  id: string;
  label: string;
}

export interface ProviderKeyDefinition {
  key: string;
  label: string;
  placeholder: string;
  required: boolean;
  sensitive: boolean;
}

export interface ModelProviderDefinition {
  id: string;
  label: string;
  keys: ProviderKeyDefinition[];
  models: Model[];
  is_custom: boolean;
}
