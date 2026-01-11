/**
 * Mock implementation of @distri/core for testing
 */

export interface DistriClientConfig {
  baseUrl: string;
  apiKey?: string;
}

export class DistriClient {
  public baseUrl: string;
  private apiKey?: string;
  private mockFetch: typeof fetch;

  constructor(config: DistriClientConfig) {
    this.baseUrl = config.baseUrl;
    this.apiKey = config.apiKey;
    // Default mock fetch that can be overridden
    this.mockFetch = globalThis.fetch;
  }

  /**
   * Make an authenticated fetch request
   */
  async fetch(path: string, options?: RequestInit): Promise<Response> {
    const url = `${this.baseUrl}${path}`;
    const headers = new Headers(options?.headers);

    if (this.apiKey) {
      headers.set('Authorization', `Bearer ${this.apiKey}`);
    }

    return this.mockFetch(url, {
      ...options,
      headers,
    });
  }

  /**
   * Set a custom fetch implementation (for testing)
   */
  setFetch(fetchImpl: typeof fetch): void {
    this.mockFetch = fetchImpl;
  }
}
