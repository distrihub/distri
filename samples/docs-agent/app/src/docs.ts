// The portal's pages. In your portal these come from your MDX or CMS.
export interface DocPage {
  slug: string;
  title: string;
  body: string;
}

export const DOCS: DocPage[] = [
  {
    slug: 'quickstart',
    title: 'Quickstart',
    body: 'Create an API key under Settings → API keys. Send it in the Authorization header: curl -H "Authorization: Bearer $ACME_KEY" https://api.acme.dev/v1/ping returns {"ok":true}.',
  },
  {
    slug: 'rate-limits',
    title: 'Rate limits',
    body: 'Each key may send 60 requests per minute. Over the limit the API answers 429 with a Retry-After header in seconds. Ask support to raise the limit.',
  },
  {
    slug: 'webhooks',
    title: 'Webhooks',
    body: 'Add an endpoint under Settings → Webhooks. Every delivery carries an Acme-Signature header: the HMAC-SHA256 of the raw body with your endpoint secret. Failed deliveries are retried 5 times over an hour.',
  },
];

/** Sections whose title or body share a word of 4+ letters with the query. */
export function searchDocs(query: string): DocPage[] {
  const words = query.toLowerCase().match(/[a-z0-9-]{4,}/g) ?? [];
  return DOCS.map((d) => {
    const text = `${d.title} ${d.body}`.toLowerCase();
    return { d, score: words.filter((w) => text.includes(w)).length };
  })
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score)
    .map((x) => x.d);
}
