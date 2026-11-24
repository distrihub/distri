// @distri/runtime — the script SDK a workflow `code` step uses to reach its run
// (distrihub/distri-cloud#304, phase 3).
//
//   import { distri } from "@distri/runtime";
//   await distri.state.set({ last_digest: Date.now() });
//   await distri.notify("3 drafts are ready", [[{ type: "url", label: "Open", url }]]);
//   const r = await distri.tool("hackernews_search", { query: "agents" });
//   const draft = await distri.agent("outreach_writer").run("draft a reply to …");
//   await distri.memory.signal({ kind: "approved", subject: "draft r1y37fgv" });
//
// Every call goes to the host that runs the workflow (`POST <url>/call`) with the step's run
// token: a short-lived token bound to this run and step, which dies when the step ends. The host
// runs the call as the run — tools get their connection tokens on the host, never here — and
// records it on the step, so the run's graph shows what the script did. The harness (`run.ts`)
// sets the URL and the token; without them (a host with no SDK) every call throws.

/** The harness sets this before it imports the module. */
interface SdkAccess {
  url: string;
  token: string;
}

const KEY = "__distri_sdk__";

function access(): SdkAccess {
  const a = (globalThis as Record<string, unknown>)[KEY] as SdkAccess | undefined;
  if (!a?.url || !a?.token) {
    throw new Error("@distri/runtime is not available here: this host offers no script SDK");
  }
  return a;
}

/** A call the host refused or that failed (`code`: `unauthorized`, `invalid`, `failed`). */
export class DistriCallError extends Error {
  constructor(message: string, readonly code: string) {
    super(message);
    this.name = "DistriCallError";
  }
}

async function call(op: string, args: Record<string, unknown> = {}): Promise<unknown> {
  const { url, token } = access();
  const res = await fetch(`${url.replace(/\/+$/, "")}/call`, {
    method: "POST",
    headers: { "authorization": `Bearer ${token}`, "content-type": "application/json" },
    body: JSON.stringify({ op, ...args }),
  });
  const text = await res.text();
  let body: { ok?: boolean; result?: unknown; error?: string; code?: string } = {};
  try {
    body = JSON.parse(text);
  } catch {
    throw new DistriCallError(`distri.${op}: the host answered ${res.status}: ${text.slice(0, 200)}`, "failed");
  }
  if (!res.ok || body.ok === false) {
    throw new DistriCallError(`distri.${op}: ${body.error ?? res.status}`, body.code ?? "failed");
  }
  return body.result;
}

/** A chat button: a callback (goes to the workflow's callback entry points) or a link. */
export type Button =
  | { type: "callback"; label: string; callback_data: string }
  | { type: "url"; label: string; url: string };

/** Fields of a memory signal (the `memory` step's signal fields). */
export interface Signal {
  kind: "approved" | "edited" | "skipped" | "outcome" | "note" | string;
  subject?: string;
  before?: string;
  after?: string;
  context?: unknown;
  space?: string;
  scope?: string;
}

export const distri = {
  /** Send the workflow's owner a message (rows of buttons, top to bottom). */
  notify(text: string, buttons?: Button[][]): Promise<unknown> {
    return call("notify", { text, buttons: buttons ?? [] });
  },
  state: {
    /** The workflow's state document (kept between runs; `{state.x}` in templates). */
    get(): Promise<Record<string, unknown>> {
      return call("state_get") as Promise<Record<string, unknown>>;
    },
    /** Shallow-merge `patch` into the state (`null` removes a key); returns the new state. */
    set(patch: Record<string, unknown>): Promise<Record<string, unknown>> {
      return call("state_set", { patch }) as Promise<Record<string, unknown>>;
    },
  },
  /** Run one of the workflow's tools on the host (connection tokens are added there). */
  tool(name: string, input: unknown = {}): Promise<unknown> {
    return call("tool", { name, input });
  },
  /** An agent (or another workflow), run as a child of this run. */
  agent(name: string) {
    return {
      run(prompt: string): Promise<unknown> {
        return call("agent", { agent: name, prompt });
      },
    };
  },
  memory: {
    /** Record how an output was received (approved, edited, skipped, …). */
    signal(signal: Signal): Promise<unknown> {
      return call("memory_signal", { signal });
    },
  },
};

export default distri;
