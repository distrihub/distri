import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  DistriHomeClient,
  HomeStats,
  AgentUsageInfo,
  ApiKey,
  Secret,
  ProviderSecretDefinition,
  PromptTemplate,
  SessionSummary,
  AgentValidationResult,
} from './DistriHomeClient';
import { DistriClient, DistriClientConfig } from '@distri/core';

/**
 * Create a mock response helper
 */
function createMockResponse<T>(data: T, ok = true, status = 200, statusText = 'OK'): Response {
  return {
    ok,
    status,
    statusText,
    json: () => Promise.resolve(data),
    headers: new Headers(),
    redirected: false,
    type: 'basic',
    url: '',
    clone: () => createMockResponse(data, ok, status, statusText),
    body: null,
    bodyUsed: false,
    arrayBuffer: () => Promise.resolve(new ArrayBuffer(0)),
    blob: () => Promise.resolve(new Blob()),
    formData: () => Promise.resolve(new FormData()),
    text: () => Promise.resolve(JSON.stringify(data)),
  } as Response;
}

/**
 * Create a mock error response
 */
function createErrorResponse(status: number, statusText: string): Response {
  return createMockResponse({}, false, status, statusText);
}

describe('DistriHomeClient', () => {
  let client: DistriHomeClient;
  let mockFetch: ReturnType<typeof vi.fn>;
  let distriClient: DistriClient;

  beforeEach(() => {
    mockFetch = vi.fn();
    globalThis.fetch = mockFetch;

    const config: DistriClientConfig = {
      baseUrl: 'https://api.distri.test',
      apiKey: 'test-api-key',
    };

    distriClient = new DistriClient(config);
    client = new DistriHomeClient(distriClient);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('constructor', () => {
    it('should accept a DistriClient instance', () => {
      const newClient = new DistriHomeClient(distriClient);
      expect(newClient.distriClient).toBe(distriClient);
    });

    it('should accept a DistriClientConfig and create internal client', () => {
      const config: DistriClientConfig = {
        baseUrl: 'https://api.distri.test',
        apiKey: 'new-api-key',
      };
      const newClient = new DistriHomeClient(config);
      expect(newClient.baseUrl).toBe('https://api.distri.test');
    });

    it('should expose the underlying DistriClient', () => {
      expect(client.distriClient).toBe(distriClient);
    });

    it('should expose the base URL', () => {
      expect(client.baseUrl).toBe('https://api.distri.test');
    });
  });

  // ============================================================
  // HOME STATS TESTS
  // ============================================================
  describe('getHomeStats', () => {
    const mockHomeStats: HomeStats = {
      total_agents: 5,
      total_owned_agents: 3,
      total_accessible_agents: 5,
      total_threads: 100,
      total_messages: 1500,
      avg_run_time_ms: 2500,
      latest_threads: [
        {
          id: 'thread-1',
          title: 'Test Thread',
          agent_id: 'agent-1',
          agent_name: 'Test Agent',
          updated_at: '2024-01-15T10:00:00Z',
          message_count: 10,
          last_message: 'Hello world',
        },
      ],
      most_active_agent: {
        id: 'agent-1',
        name: 'Most Active Agent',
        count: 50,
      },
      recently_used_agents: [
        {
          id: 'agent-1',
          name: 'Recent Agent',
          description: 'A recently used agent',
          last_used_at: '2024-01-15T10:00:00Z',
        },
      ],
      custom_metrics: {
        api_calls: {
          label: 'API Calls',
          value: '1,000',
          helper: 'Total API calls this month',
          limit: '10,000',
          raw_value: 1000,
          raw_limit: 10000,
        },
      },
    };

    it('should fetch home stats successfully', async () => {
      mockFetch.mockResolvedValueOnce(createMockResponse(mockHomeStats));

      const result = await client.getHomeStats();

      expect(mockFetch).toHaveBeenCalledWith(
        'https://api.distri.test/home/stats',
        expect.objectContaining({
          headers: expect.any(Headers),
        })
      );
      expect(result).toEqual(mockHomeStats);
    });

    it('should handle home stats with minimal data', async () => {
      const minimalStats: HomeStats = {
        total_agents: 0,
        total_threads: 0,
      };
      mockFetch.mockResolvedValueOnce(createMockResponse(minimalStats));

      const result = await client.getHomeStats();

      expect(result.total_agents).toBe(0);
      expect(result.total_threads).toBe(0);
      expect(result.latest_threads).toBeUndefined();
    });

    it('should throw error when fetch fails', async () => {
      mockFetch.mockResolvedValueOnce(createErrorResponse(500, 'Internal Server Error'));

      await expect(client.getHomeStats()).rejects.toThrow(
        'Failed to fetch home stats: Internal Server Error'
      );
    });

    it('should throw error on 401 unauthorized', async () => {
      mockFetch.mockResolvedValueOnce(createErrorResponse(401, 'Unauthorized'));

      await expect(client.getHomeStats()).rejects.toThrow(
        'Failed to fetch home stats: Unauthorized'
      );
    });

    it('should throw error on network failure', async () => {
      mockFetch.mockRejectedValueOnce(new Error('Network error'));

      await expect(client.getHomeStats()).rejects.toThrow('Network error');
    });
  });

  // ============================================================
  // AGENTS BY USAGE TESTS
  // ============================================================
  describe('getAgentsByUsage', () => {
    const mockAgentUsage: AgentUsageInfo[] = [
      { agent_id: 'agent-1', agent_name: 'Popular Agent', thread_count: 100 },
      { agent_id: 'agent-2', agent_name: 'Second Agent', thread_count: 50 },
      { agent_id: 'agent-3', agent_name: 'Third Agent', thread_count: 25 },
    ];

    it('should fetch agents sorted by usage', async () => {
      mockFetch.mockResolvedValueOnce(createMockResponse(mockAgentUsage));

      const result = await client.getAgentsByUsage();

      expect(mockFetch).toHaveBeenCalledWith(
        'https://api.distri.test/agents/usage',
        expect.objectContaining({
          headers: expect.any(Headers),
        })
      );
      expect(result).toEqual(mockAgentUsage);
      expect(result).toHaveLength(3);
      expect(result[0].thread_count).toBeGreaterThan(result[1].thread_count);
    });

    it('should return empty array when no agents', async () => {
      mockFetch.mockResolvedValueOnce(createMockResponse([]));

      const result = await client.getAgentsByUsage();

      expect(result).toEqual([]);
      expect(result).toHaveLength(0);
    });

    it('should throw error when fetch fails', async () => {
      mockFetch.mockResolvedValueOnce(createErrorResponse(500, 'Internal Server Error'));

      await expect(client.getAgentsByUsage()).rejects.toThrow(
        'Failed to fetch agents by usage: Internal Server Error'
      );
    });
  });

  // ============================================================
  // API KEYS TESTS
  // ============================================================
  describe('API Keys', () => {
    const mockApiKeys: ApiKey[] = [
      {
        id: 'key-1',
        label: 'Production Key',
        name: 'prod-key',
        key: 'sk_live_abc123',
        created_at: '2024-01-01T00:00:00Z',
      },
      {
        id: 'key-2',
        label: 'Development Key',
        name: 'dev-key',
        key: 'sk_test_xyz789',
        created_at: '2024-01-10T00:00:00Z',
      },
    ];

    describe('listApiKeys', () => {
      it('should list all API keys', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(mockApiKeys));

        const result = await client.listApiKeys();

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/api-keys',
          expect.objectContaining({
            headers: expect.any(Headers),
          })
        );
        expect(result).toEqual(mockApiKeys);
        expect(result).toHaveLength(2);
      });

      it('should return empty array when no keys exist', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse([]));

        const result = await client.listApiKeys();

        expect(result).toEqual([]);
      });

      it('should throw error when fetch fails', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(500, 'Internal Server Error'));

        await expect(client.listApiKeys()).rejects.toThrow(
          'Failed to fetch API keys: Internal Server Error'
        );
      });
    });

    describe('createApiKey', () => {
      const newKey: ApiKey = {
        id: 'key-3',
        label: 'New API Key',
        name: 'new-key',
        key: 'sk_live_newkey123',
        created_at: '2024-01-15T00:00:00Z',
      };

      it('should create a new API key', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(newKey));

        const result = await client.createApiKey('New API Key');

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/api-keys',
          expect.objectContaining({
            method: 'POST',
            headers: expect.any(Headers),
            body: JSON.stringify({ label: 'New API Key' }),
          })
        );
        expect(result).toEqual(newKey);
        expect(result.label).toBe('New API Key');
      });

      it('should create key with empty label', async () => {
        const keyWithEmptyLabel: ApiKey = { ...newKey, label: '' };
        mockFetch.mockResolvedValueOnce(createMockResponse(keyWithEmptyLabel));

        const result = await client.createApiKey('');

        expect(result.label).toBe('');
      });

      it('should throw error when creation fails', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(400, 'Bad Request'));

        await expect(client.createApiKey('Invalid Key')).rejects.toThrow(
          'Failed to create API key: Bad Request'
        );
      });

      it('should throw error on rate limit', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(429, 'Too Many Requests'));

        await expect(client.createApiKey('Another Key')).rejects.toThrow(
          'Failed to create API key: Too Many Requests'
        );
      });
    });

    describe('revokeApiKey', () => {
      it('should revoke an API key', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(null, true, 204, 'No Content'));

        await expect(client.revokeApiKey('key-1')).resolves.toBeUndefined();

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/api-keys/key-1',
          expect.objectContaining({
            method: 'DELETE',
            headers: expect.any(Headers),
          })
        );
      });

      it('should throw error when key not found', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(404, 'Not Found'));

        await expect(client.revokeApiKey('nonexistent-key')).rejects.toThrow(
          'Failed to revoke API key: Not Found'
        );
      });

      it('should throw error when revocation fails', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(500, 'Internal Server Error'));

        await expect(client.revokeApiKey('key-1')).rejects.toThrow(
          'Failed to revoke API key: Internal Server Error'
        );
      });
    });
  });

  // ============================================================
  // SECRETS TESTS
  // ============================================================
  describe('Secrets', () => {
    const mockSecrets: Secret[] = [
      {
        id: 'secret-1',
        key: 'OPENAI_API_KEY',
        masked_value: 'sk-****abc',
        created_at: '2024-01-01T00:00:00Z',
        updated_at: '2024-01-10T00:00:00Z',
      },
      {
        id: 'secret-2',
        key: 'ANTHROPIC_API_KEY',
        masked_value: 'sk-ant-****xyz',
        created_at: '2024-01-05T00:00:00Z',
        updated_at: '2024-01-05T00:00:00Z',
      },
    ];

    describe('listSecrets', () => {
      it('should list all secrets', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(mockSecrets));

        const result = await client.listSecrets();

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/secrets',
          expect.objectContaining({
            headers: expect.any(Headers),
          })
        );
        expect(result).toEqual(mockSecrets);
        expect(result).toHaveLength(2);
      });

      it('should return empty array when no secrets exist', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse([]));

        const result = await client.listSecrets();

        expect(result).toEqual([]);
      });

      it('should verify secrets have masked values', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(mockSecrets));

        const result = await client.listSecrets();

        result.forEach((secret) => {
          expect(secret.masked_value).toContain('****');
        });
      });

      it('should throw error when fetch fails', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(500, 'Internal Server Error'));

        await expect(client.listSecrets()).rejects.toThrow(
          'Failed to fetch secrets: Internal Server Error'
        );
      });
    });

    describe('createSecret', () => {
      const newSecret: Secret = {
        id: 'secret-3',
        key: 'NEW_SECRET_KEY',
        masked_value: '****new',
        created_at: '2024-01-15T00:00:00Z',
        updated_at: '2024-01-15T00:00:00Z',
      };

      it('should create a new secret', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(newSecret));

        const result = await client.createSecret('NEW_SECRET_KEY', 'secret-value-123');

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/secrets',
          expect.objectContaining({
            method: 'POST',
            headers: expect.any(Headers),
            body: JSON.stringify({ key: 'NEW_SECRET_KEY', value: 'secret-value-123' }),
          })
        );
        expect(result).toEqual(newSecret);
        expect(result.key).toBe('NEW_SECRET_KEY');
      });

      it('should update an existing secret (upsert behavior)', async () => {
        const updatedSecret: Secret = {
          ...mockSecrets[0],
          updated_at: '2024-01-15T12:00:00Z',
        };
        mockFetch.mockResolvedValueOnce(createMockResponse(updatedSecret));

        const result = await client.createSecret('OPENAI_API_KEY', 'new-api-key-value');

        expect(result.key).toBe('OPENAI_API_KEY');
        expect(result.updated_at).toBe('2024-01-15T12:00:00Z');
      });

      it('should throw error when creation fails', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(400, 'Bad Request'));

        await expect(client.createSecret('INVALID', 'value')).rejects.toThrow(
          'Failed to create secret: Bad Request'
        );
      });

      it('should throw error on validation failure', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(422, 'Unprocessable Entity'));

        await expect(client.createSecret('', '')).rejects.toThrow(
          'Failed to create secret: Unprocessable Entity'
        );
      });
    });

    describe('deleteSecret', () => {
      it('should delete a secret', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(null, true, 204, 'No Content'));

        await expect(client.deleteSecret('secret-1')).resolves.toBeUndefined();

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/secrets/secret-1',
          expect.objectContaining({
            method: 'DELETE',
            headers: expect.any(Headers),
          })
        );
      });

      it('should throw error when secret not found', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(404, 'Not Found'));

        await expect(client.deleteSecret('nonexistent')).rejects.toThrow(
          'Failed to delete secret: Not Found'
        );
      });

      it('should throw error when deletion fails', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(500, 'Internal Server Error'));

        await expect(client.deleteSecret('secret-1')).rejects.toThrow(
          'Failed to delete secret: Internal Server Error'
        );
      });
    });

    describe('listProviderDefinitions', () => {
      const mockProviders: ProviderSecretDefinition[] = [
        {
          id: 'openai',
          label: 'OpenAI',
          keys: [
            {
              key: 'OPENAI_API_KEY',
              label: 'API Key',
              placeholder: 'sk-...',
              required: true,
            },
          ],
        },
        {
          id: 'anthropic',
          label: 'Anthropic',
          keys: [
            {
              key: 'ANTHROPIC_API_KEY',
              label: 'API Key',
              placeholder: 'sk-ant-...',
              required: true,
            },
          ],
        },
        {
          id: 'aws',
          label: 'AWS Bedrock',
          keys: [
            {
              key: 'AWS_ACCESS_KEY_ID',
              label: 'Access Key ID',
              placeholder: 'AKIA...',
              required: true,
            },
            {
              key: 'AWS_SECRET_ACCESS_KEY',
              label: 'Secret Access Key',
              placeholder: 'Your secret key',
              required: true,
            },
            {
              key: 'AWS_REGION',
              label: 'Region',
              placeholder: 'us-east-1',
              required: false,
            },
          ],
        },
      ];

      it('should list provider definitions', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(mockProviders));

        const result = await client.listProviderDefinitions();

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/secrets/providers',
          expect.objectContaining({
            headers: expect.any(Headers),
          })
        );
        expect(result).toEqual(mockProviders);
        expect(result).toHaveLength(3);
      });

      it('should include required/optional key information', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(mockProviders));

        const result = await client.listProviderDefinitions();
        const awsProvider = result.find((p) => p.id === 'aws');

        expect(awsProvider).toBeDefined();
        expect(awsProvider?.keys).toHaveLength(3);
        expect(awsProvider?.keys[0].required).toBe(true);
        expect(awsProvider?.keys[2].required).toBe(false);
      });

      it('should return empty array when no providers defined', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse([]));

        const result = await client.listProviderDefinitions();

        expect(result).toEqual([]);
      });

      it('should throw error when fetch fails', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(500, 'Internal Server Error'));

        await expect(client.listProviderDefinitions()).rejects.toThrow(
          'Failed to fetch provider definitions: Internal Server Error'
        );
      });
    });
  });

  // ============================================================
  // PROMPT TEMPLATES TESTS
  // ============================================================
  describe('Prompt Templates', () => {
    const mockTemplates: PromptTemplate[] = [
      {
        id: 'template-1',
        name: 'Customer Support',
        template: 'You are a helpful customer support agent. {{context}}',
        description: 'Template for customer support agents',
        version: '1.0',
        is_system: true,
        created_at: '2024-01-01T00:00:00Z',
        updated_at: '2024-01-01T00:00:00Z',
      },
      {
        id: 'template-2',
        name: 'Code Assistant',
        template: 'You are an expert programmer. Help with: {{task}}',
        description: 'Template for coding assistance',
        version: '1.0',
        is_system: false,
        created_at: '2024-01-05T00:00:00Z',
        updated_at: '2024-01-10T00:00:00Z',
      },
    ];

    describe('listPromptTemplates', () => {
      it('should list all prompt templates', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(mockTemplates));

        const result = await client.listPromptTemplates();

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/prompt-templates',
          expect.objectContaining({
            headers: expect.any(Headers),
          })
        );
        expect(result).toEqual(mockTemplates);
        expect(result).toHaveLength(2);
      });

      it('should include both system and user templates', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(mockTemplates));

        const result = await client.listPromptTemplates();

        const systemTemplates = result.filter((t) => t.is_system);
        const userTemplates = result.filter((t) => !t.is_system);

        expect(systemTemplates).toHaveLength(1);
        expect(userTemplates).toHaveLength(1);
      });

      it('should return empty array when no templates exist', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse([]));

        const result = await client.listPromptTemplates();

        expect(result).toEqual([]);
      });

      it('should throw error when fetch fails', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(500, 'Internal Server Error'));

        await expect(client.listPromptTemplates()).rejects.toThrow(
          'Failed to fetch prompt templates: Internal Server Error'
        );
      });
    });

    describe('createPromptTemplate', () => {
      const newTemplate: PromptTemplate = {
        id: 'template-3',
        name: 'New Template',
        template: 'This is a new template with {{variable}}',
        is_system: false,
        created_at: '2024-01-15T00:00:00Z',
        updated_at: '2024-01-15T00:00:00Z',
      };

      it('should create a new prompt template', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(newTemplate));

        const result = await client.createPromptTemplate(
          'New Template',
          'This is a new template with {{variable}}'
        );

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/prompt-templates',
          expect.objectContaining({
            method: 'POST',
            headers: expect.any(Headers),
            body: JSON.stringify({
              name: 'New Template',
              template: 'This is a new template with {{variable}}',
            }),
          })
        );
        expect(result).toEqual(newTemplate);
      });

      it('should create template with special characters', async () => {
        const templateWithSpecialChars: PromptTemplate = {
          ...newTemplate,
          template: 'Handle "quotes" and <tags> properly {{var}}',
        };
        mockFetch.mockResolvedValueOnce(createMockResponse(templateWithSpecialChars));

        const result = await client.createPromptTemplate(
          'Special Chars Template',
          'Handle "quotes" and <tags> properly {{var}}'
        );

        expect(result.template).toContain('"quotes"');
        expect(result.template).toContain('<tags>');
      });

      it('should throw error when creation fails', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(400, 'Bad Request'));

        await expect(client.createPromptTemplate('Invalid', '')).rejects.toThrow(
          'Failed to create prompt template: Bad Request'
        );
      });
    });

    describe('updatePromptTemplate', () => {
      const updatedTemplate: PromptTemplate = {
        ...mockTemplates[1],
        name: 'Updated Code Assistant',
        template: 'You are an expert programmer. New instructions: {{task}}',
        updated_at: '2024-01-15T12:00:00Z',
      };

      it('should update an existing prompt template', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(updatedTemplate));

        const result = await client.updatePromptTemplate(
          'template-2',
          'Updated Code Assistant',
          'You are an expert programmer. New instructions: {{task}}'
        );

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/prompt-templates/template-2',
          expect.objectContaining({
            method: 'PUT',
            headers: expect.any(Headers),
            body: JSON.stringify({
              name: 'Updated Code Assistant',
              template: 'You are an expert programmer. New instructions: {{task}}',
            }),
          })
        );
        expect(result.name).toBe('Updated Code Assistant');
      });

      it('should throw error when template not found', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(404, 'Not Found'));

        await expect(
          client.updatePromptTemplate('nonexistent', 'Name', 'Template')
        ).rejects.toThrow('Failed to update prompt template: Not Found');
      });

      it('should throw error when trying to update system template', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(403, 'Forbidden'));

        await expect(
          client.updatePromptTemplate('template-1', 'New Name', 'New Template')
        ).rejects.toThrow('Failed to update prompt template: Forbidden');
      });
    });

    describe('deletePromptTemplate', () => {
      it('should delete a prompt template', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(null, true, 204, 'No Content'));

        await expect(client.deletePromptTemplate('template-2')).resolves.toBeUndefined();

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/prompt-templates/template-2',
          expect.objectContaining({
            method: 'DELETE',
            headers: expect.any(Headers),
          })
        );
      });

      it('should throw error when template not found', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(404, 'Not Found'));

        await expect(client.deletePromptTemplate('nonexistent')).rejects.toThrow(
          'Failed to delete prompt template: Not Found'
        );
      });

      it('should throw error when trying to delete system template', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(403, 'Forbidden'));

        await expect(client.deletePromptTemplate('template-1')).rejects.toThrow(
          'Failed to delete prompt template: Forbidden'
        );
      });
    });

    describe('clonePromptTemplate', () => {
      const clonedTemplate: PromptTemplate = {
        id: 'template-4',
        name: 'Customer Support (Copy)',
        template: 'You are a helpful customer support agent. {{context}}',
        description: 'Template for customer support agents',
        version: '1.0',
        is_system: false,
        created_at: '2024-01-15T00:00:00Z',
        updated_at: '2024-01-15T00:00:00Z',
      };

      it('should clone a prompt template', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(clonedTemplate));

        const result = await client.clonePromptTemplate('template-1');

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/prompt-templates/template-1/clone',
          expect.objectContaining({
            method: 'POST',
            headers: expect.any(Headers),
          })
        );
        expect(result.id).not.toBe('template-1');
        expect(result.is_system).toBe(false);
      });

      it('should create a non-system copy of system template', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(clonedTemplate));

        const result = await client.clonePromptTemplate('template-1');

        expect(result.is_system).toBe(false);
      });

      it('should throw error when source template not found', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(404, 'Not Found'));

        await expect(client.clonePromptTemplate('nonexistent')).rejects.toThrow(
          'Failed to clone prompt template: Not Found'
        );
      });
    });
  });

  // ============================================================
  // SESSIONS TESTS
  // ============================================================
  describe('Sessions', () => {
    const mockSessions: SessionSummary[] = [
      {
        session_id: 'session-1',
        keys: ['user_id', 'preferences', 'history'],
        key_count: 3,
        updated_at: '2024-01-15T10:00:00Z',
      },
      {
        session_id: 'session-2',
        keys: ['cart_items', 'user_id'],
        key_count: 2,
        updated_at: '2024-01-14T15:00:00Z',
      },
    ];

    describe('listSessions', () => {
      it('should list sessions without options', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(mockSessions));

        const result = await client.listSessions();

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/sessions?',
          expect.objectContaining({
            headers: expect.any(Headers),
          })
        );
        expect(result).toEqual(mockSessions);
      });

      it('should list sessions with threadId filter', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse([mockSessions[0]]));

        const result = await client.listSessions({ threadId: 'thread-123' });

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/sessions?thread_id=thread-123',
          expect.objectContaining({
            headers: expect.any(Headers),
          })
        );
        expect(result).toHaveLength(1);
      });

      it('should list sessions with pagination', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(mockSessions));

        const result = await client.listSessions({ limit: 10, offset: 20 });

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/sessions?limit=10&offset=20',
          expect.objectContaining({
            headers: expect.any(Headers),
          })
        );
        expect(result).toEqual(mockSessions);
      });

      it('should list sessions with all options combined', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse([mockSessions[0]]));

        const result = await client.listSessions({
          threadId: 'thread-123',
          limit: 5,
          offset: 10,
        });

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/sessions?thread_id=thread-123&limit=5&offset=10',
          expect.objectContaining({
            headers: expect.any(Headers),
          })
        );
        expect(result).toHaveLength(1);
      });

      it('should return empty array when no sessions exist', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse([]));

        const result = await client.listSessions();

        expect(result).toEqual([]);
      });

      it('should throw error when fetch fails', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(500, 'Internal Server Error'));

        await expect(client.listSessions()).rejects.toThrow(
          'Failed to list sessions: Internal Server Error'
        );
      });
    });

    describe('getSessionValues', () => {
      const mockSessionValues = {
        values: {
          user_id: 'user-123',
          preferences: { theme: 'dark', language: 'en' },
          history: ['action1', 'action2', 'action3'],
        },
      };

      it('should get session values', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse(mockSessionValues));

        const result = await client.getSessionValues('session-1');

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/sessions/session-1/values',
          expect.objectContaining({
            headers: expect.any(Headers),
          })
        );
        expect(result).toEqual(mockSessionValues.values);
        expect(result.user_id).toBe('user-123');
      });

      it('should handle complex nested values', async () => {
        const complexValues = {
          values: {
            nested: {
              deeply: {
                nested: {
                  value: 'found',
                },
              },
            },
            array: [1, 2, { nested: 'value' }],
          },
        };
        mockFetch.mockResolvedValueOnce(createMockResponse(complexValues));

        const result = await client.getSessionValues('session-1');

        expect(result.nested.deeply.nested.value).toBe('found');
        expect(result.array).toHaveLength(3);
      });

      it('should return empty object when session has no values', async () => {
        mockFetch.mockResolvedValueOnce(createMockResponse({ values: {} }));

        const result = await client.getSessionValues('empty-session');

        expect(result).toEqual({});
      });

      it('should throw error when session not found', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(404, 'Not Found'));

        await expect(client.getSessionValues('nonexistent')).rejects.toThrow(
          'Failed to get session values: Not Found'
        );
      });

      it('should throw error when fetch fails', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(500, 'Internal Server Error'));

        await expect(client.getSessionValues('session-1')).rejects.toThrow(
          'Failed to get session values: Internal Server Error'
        );
      });
    });
  });

  // ============================================================
  // AGENT VALIDATION TESTS
  // ============================================================
  describe('Agent Validation', () => {
    describe('validateAgent', () => {
      it('should validate agent successfully with no warnings', async () => {
        const validResult: AgentValidationResult = {
          valid: true,
          warnings: [],
        };
        mockFetch.mockResolvedValueOnce(createMockResponse(validResult));

        const result = await client.validateAgent('agent-1');

        expect(mockFetch).toHaveBeenCalledWith(
          'https://api.distri.test/agents/agent-1/validate',
          expect.objectContaining({
            headers: expect.any(Headers),
          })
        );
        expect(result.valid).toBe(true);
        expect(result.warnings).toHaveLength(0);
      });

      it('should return warnings for missing secrets', async () => {
        const resultWithWarnings: AgentValidationResult = {
          valid: false,
          warnings: [
            {
              code: 'MISSING_SECRET',
              message: 'Required secret OPENAI_API_KEY is not configured',
              severity: 'error',
            },
            {
              code: 'MISSING_SECRET',
              message: 'Optional secret CUSTOM_API_KEY is not configured',
              severity: 'warning',
            },
          ],
        };
        mockFetch.mockResolvedValueOnce(createMockResponse(resultWithWarnings));

        const result = await client.validateAgent('agent-with-issues');

        expect(result.valid).toBe(false);
        expect(result.warnings).toHaveLength(2);
        expect(result.warnings[0].severity).toBe('error');
        expect(result.warnings[1].severity).toBe('warning');
      });

      it('should handle different warning severities', async () => {
        const mixedWarnings: AgentValidationResult = {
          valid: false,
          warnings: [
            {
              code: 'CONFIG_ERROR',
              message: 'Invalid configuration',
              severity: 'error',
            },
            {
              code: 'DEPRECATED_FEATURE',
              message: 'Using deprecated feature',
              severity: 'warning',
            },
          ],
        };
        mockFetch.mockResolvedValueOnce(createMockResponse(mixedWarnings));

        const result = await client.validateAgent('agent-1');

        const errors = result.warnings.filter((w) => w.severity === 'error');
        const warnings = result.warnings.filter((w) => w.severity === 'warning');

        expect(errors).toHaveLength(1);
        expect(warnings).toHaveLength(1);
      });

      it('should throw error when agent not found', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(404, 'Not Found'));

        await expect(client.validateAgent('nonexistent-agent')).rejects.toThrow(
          'Failed to validate agent: Not Found'
        );
      });

      it('should throw error when validation service fails', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(500, 'Internal Server Error'));

        await expect(client.validateAgent('agent-1')).rejects.toThrow(
          'Failed to validate agent: Internal Server Error'
        );
      });

      it('should throw error on 401 unauthorized', async () => {
        mockFetch.mockResolvedValueOnce(createErrorResponse(401, 'Unauthorized'));

        await expect(client.validateAgent('agent-1')).rejects.toThrow(
          'Failed to validate agent: Unauthorized'
        );
      });
    });
  });

  // ============================================================
  // EDGE CASES AND ERROR HANDLING
  // ============================================================
  describe('Edge Cases and Error Handling', () => {
    it('should handle special characters in IDs', async () => {
      mockFetch.mockResolvedValueOnce(createMockResponse(null, true, 204, 'No Content'));

      await client.revokeApiKey('key-with-special-chars-123');

      expect(mockFetch).toHaveBeenCalledWith(
        'https://api.distri.test/api-keys/key-with-special-chars-123',
        expect.any(Object)
      );
    });

    it('should handle unicode in template content', async () => {
      const unicodeTemplate: PromptTemplate = {
        id: 'unicode-template',
        name: '日本語テンプレート',
        template: 'こんにちは {{name}} さん！ 🎉',
        is_system: false,
        created_at: '2024-01-15T00:00:00Z',
        updated_at: '2024-01-15T00:00:00Z',
      };
      mockFetch.mockResolvedValueOnce(createMockResponse(unicodeTemplate));

      const result = await client.createPromptTemplate('日本語テンプレート', 'こんにちは {{name}} さん！ 🎉');

      expect(result.name).toBe('日本語テンプレート');
      expect(result.template).toContain('🎉');
    });

    it('should handle very long strings', async () => {
      const longString = 'a'.repeat(10000);
      const templateWithLongContent: PromptTemplate = {
        id: 'long-template',
        name: 'Long Template',
        template: longString,
        is_system: false,
        created_at: '2024-01-15T00:00:00Z',
        updated_at: '2024-01-15T00:00:00Z',
      };
      mockFetch.mockResolvedValueOnce(createMockResponse(templateWithLongContent));

      const result = await client.createPromptTemplate('Long Template', longString);

      expect(result.template).toHaveLength(10000);
    });

    it('should handle network timeout', async () => {
      mockFetch.mockRejectedValueOnce(new Error('Timeout'));

      await expect(client.getHomeStats()).rejects.toThrow('Timeout');
    });

    it('should handle malformed JSON response', async () => {
      const badResponse = {
        ok: true,
        status: 200,
        statusText: 'OK',
        json: () => Promise.reject(new Error('Invalid JSON')),
      } as Response;
      mockFetch.mockResolvedValueOnce(badResponse);

      await expect(client.getHomeStats()).rejects.toThrow('Invalid JSON');
    });
  });
});
