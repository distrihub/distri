import type { Plugin } from 'vite';

// Dev-server endpoint: exchanges the long-lived DISTRI_API_KEY for a
// short-lived token (POST {baseUrl}/token with x-api-key). In production, put
// the same exchange in your backend; never ship the API key to the browser.
export function distriToken(opts: { apiKey: string; baseUrl: string; workspaceId?: string }): Plugin {
  return {
    name: 'distri-token',
    configureServer(server) {
      server.middlewares.use('/api/distri/token', async (_req, res) => {
        res.setHeader('content-type', 'application/json');
        if (!opts.apiKey) {
          res.statusCode = 500;
          res.end(JSON.stringify({ error: 'Set DISTRI_API_KEY in app/.env' }));
          return;
        }
        const r = await fetch(`${opts.baseUrl.replace(/\/$/, '')}/token`, {
          method: 'POST',
          headers: { 'x-api-key': opts.apiKey, 'content-type': 'application/json' },
          body: JSON.stringify({ ttl: 3600 }),
        });
        const body = (await r.json().catch(() => ({}))) as { access_token?: string };
        if (!r.ok || !body.access_token) {
          res.statusCode = 502;
          res.end(JSON.stringify({ error: `token exchange returned ${r.status}` }));
          return;
        }
        res.end(JSON.stringify({ access_token: body.access_token, base_url: opts.baseUrl, workspace_id: opts.workspaceId }));
      });
    },
  };
}
