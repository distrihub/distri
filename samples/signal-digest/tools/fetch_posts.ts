// ---
// [permissions]
// net = ["hn.algolia.com", "www.reddit.com"]
// [tool]
// timeout = 45
// ---
// Fetch the last day's posts about a topic from Hacker News (Algolia) and Reddit, minus the
// ones already sent (`seen`). Returns the new posts and the updated `seen` list.
import { defineTool, s } from "jsr:@distri/tool";

type Post = { id: string; title: string; url: string; source: string; points: number; comments: number };

async function hackerNews(topic: string, since: number): Promise<Post[]> {
  const q = new URLSearchParams({
    query: topic,
    tags: "story",
    numericFilters: `created_at_i>${since}`,
    hitsPerPage: "40",
  });
  const res = await fetch(`https://hn.algolia.com/api/v1/search_by_date?${q}`);
  if (!res.ok) throw new Error(`hacker news: HTTP ${res.status}`);
  const body = await res.json();
  return (body.hits ?? []).map((h: Record<string, unknown>) => ({
    id: `hn:${h.objectID}`,
    title: String(h.title ?? ""),
    url: String(h.url ?? `https://news.ycombinator.com/item?id=${h.objectID}`),
    source: "Hacker News",
    points: Number(h.points ?? 0),
    comments: Number(h.num_comments ?? 0),
  }));
}

async function reddit(topic: string): Promise<Post[]> {
  const q = new URLSearchParams({ q: topic, sort: "top", t: "day", limit: "40" });
  const res = await fetch(`https://www.reddit.com/search.json?${q}`, {
    headers: { "user-agent": "distri-signal-digest/1.0" },
  });
  if (!res.ok) return []; // Reddit rate-limits anonymous clients; Hacker News alone is fine.
  const body = await res.json();
  return (body.data?.children ?? []).map((c: { data: Record<string, unknown> }) => ({
    id: `reddit:${c.data.id}`,
    title: String(c.data.title ?? ""),
    url: `https://www.reddit.com${c.data.permalink}`,
    source: `r/${c.data.subreddit}`,
    points: Number(c.data.score ?? 0),
    comments: Number(c.data.num_comments ?? 0),
  }));
}

export const tool = defineTool({
  name: "fetch_posts",
  description: "New posts about a topic from Hacker News and Reddit, minus ones already sent",
  input: s.object({
    topic: s.string(),
    seen: s.optional(s.array(s.string())),
    hours: s.optional(s.number()),
  }),
  async run({ topic, seen, hours }, ctx) {
    const since = Math.floor(Date.now() / 1000) - (hours ?? 24) * 3600;
    const prior = Array.isArray(seen) ? seen : [];
    const already = new Set(prior);
    const [hn, rd] = await Promise.all([hackerNews(topic, since), reddit(topic)]);
    const fresh = [...hn, ...rd]
      .filter((p) => p.title && !already.has(p.id))
      .sort((a, b) => b.points + b.comments - (a.points + a.comments))
      .slice(0, 30);
    ctx.log(`hn=${hn.length} reddit=${rd.length} new=${fresh.length}`);
    const next = [...fresh.map((p) => p.id), ...prior].slice(0, 500);
    return { posts: fresh, count: fresh.length, seen_next: next };
  },
});
