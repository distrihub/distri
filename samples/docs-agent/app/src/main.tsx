import { StrictMode, useEffect, useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Chat, DistriProvider } from '@distri/react';
import '@distri/react/globals.css';
import type { DistriFnTool } from '@distri/core';
import { DOCS, searchDocs } from './docs';

// Tools the agent calls; they run here, in the reader's browser. Both only
// read the docs and switch the page, so they run without asking the reader
// (autoExecute); leave it off for tools that change data.
function docsTools(open: (slug: string) => void): DistriFnTool[] {
  return [
    {
      type: 'function',
      name: 'search_docs',
      autoExecute: true,
      description: 'Search the portal docs. Returns matching pages with slug, title and body.',
      parameters: {
        type: 'object',
        properties: { query: { type: 'string', description: 'The reader question or keywords' } },
        required: ['query'],
      },
      handler: async ({ query }: { query: string }) => JSON.stringify(searchDocs(query)),
    },
    {
      type: 'function',
      name: 'open_page',
      autoExecute: true,
      description: 'Open a docs page for the reader by its slug.',
      parameters: {
        type: 'object',
        properties: { slug: { type: 'string', enum: DOCS.map((d) => d.slug) } },
        required: ['slug'],
      },
      handler: async ({ slug }: { slug: string }) => {
        open(slug);
        return `Opened ${slug}`;
      },
    },
  ];
}

function useToken() {
  const [token, setToken] = useState<{ access_token: string; base_url: string; workspace_id?: string } | string | null>(null);
  useEffect(() => {
    fetch('/api/distri/token', { method: 'POST' })
      .then(async (r) => (r.ok ? r.json() : Promise.reject(new Error((await r.json()).error))))
      .then(setToken)
      .catch((e: Error) => setToken(e.message));
  }, []);
  return token;
}

function Portal() {
  const [slug, setSlug] = useState('quickstart');
  const [threadId] = useState(() => `docs-${Date.now()}`);
  const tools = useMemo(() => docsTools(setSlug), []);
  const token = useToken();
  const page = DOCS.find((d) => d.slug === slug)!;

  return (
    <div style={{ display: 'flex', height: '100vh', fontFamily: 'system-ui, sans-serif' }}>
      <nav style={{ width: 200, padding: 16, borderRight: '1px solid #ddd' }}>
        <strong>Acme API docs</strong>
        {DOCS.map((d) => (
          <div key={d.slug}>
            <a href="#" data-testid={`nav-${d.slug}`} onClick={() => setSlug(d.slug)} style={{ fontWeight: d.slug === slug ? 700 : 400 }}>
              {d.title}
            </a>
          </div>
        ))}
      </nav>
      <main style={{ flex: 1, padding: 24 }} data-testid="page" data-slug={slug}>
        <h1>{page.title}</h1>
        <p>{page.body}</p>
      </main>
      <aside style={{ width: 420, borderLeft: '1px solid #ddd' }}>
        {typeof token === 'string' ? (
          <p style={{ padding: 16 }}>Could not get a token: {token}</p>
        ) : token ? (
          <DistriProvider config={{ baseUrl: token.base_url, accessToken: token.access_token, workspaceId: token.workspace_id, authReady: true }}>
            <Chat agentId="docs_agent" threadId={threadId} externalTools={tools} />
          </DistriProvider>
        ) : (
          <p style={{ padding: 16 }}>Connecting…</p>
        )}
      </aside>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <Portal />
  </StrictMode>,
);
