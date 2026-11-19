// ---
// [permissions]
// net = ["api.linear.app"]
// [tool]
// timeout = 30
// ---
// File a Linear issue for a support request. The token comes from the workflow's Linear
// connection through the step's input mapping; it never reaches the website.
import { defineTool, s } from "jsr:@distri/tool";

async function linear(token: string, query: string, variables: Record<string, unknown>) {
  const res = await fetch("https://api.linear.app/graphql", {
    method: "POST",
    headers: { "content-type": "application/json", authorization: token.startsWith("lin_") ? token : `Bearer ${token}` },
    body: JSON.stringify({ query, variables }),
  });
  const body = await res.json();
  if (!res.ok || body.errors) throw new Error(`linear: ${res.status} ${JSON.stringify(body.errors ?? body)}`);
  return body.data;
}

export const tool = defineTool({
  name: "create_ticket",
  description: "Create a Linear issue for a support request",
  input: s.object({
    token: s.string(),
    team_key: s.string(),
    email: s.string(),
    summary: s.string(),
    details: s.optional(s.string()),
    page: s.optional(s.string()),
  }),
  async run({ token, team_key, email, summary, details, page }) {
    if (!token || token.startsWith("{env.")) throw new Error("Linear is not connected");
    const teams = await linear(token, "query($key: String!) { teams(filter: { key: { eq: $key } }) { nodes { id } } }", { key: team_key });
    const teamId = teams.teams.nodes[0]?.id;
    if (!teamId) throw new Error(`no Linear team with key ${team_key}`);
    const description = [`**From:** ${email}`, page ? `**Page:** ${page}` : "", "", details ?? ""].join("\n");
    const out = await linear(
      token,
      "mutation($input: IssueCreateInput!) { issueCreate(input: $input) { success issue { identifier url } } }",
      { input: { teamId, title: summary, description } },
    );
    const issue = out.issueCreate.issue;
    return { ok: true, identifier: issue.identifier, url: issue.url };
  },
});
