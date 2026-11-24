// @distri/runtime — the script SDK a workflow `code` step uses to reach its run
// (distrihub/distri-cloud#304, phase 3).
//
//   import { distri } from "@distri/runtime";
//   await distri.state.set({ last_digest: Date.now() });
//   await distri.notify("3 drafts are ready", [[{ kind: "url", label: "Open", url }]]);
//   const r = await distri.tool("hackernews_search", { query: "agents" });
//   const draft = await distri.agent("outreach_writer").run("draft a reply to …");
//   await distri.memory.signal({ kind: "approved", subject: "draft r1y37fgv" });
//
// Every call goes to the host that runs the workflow, which runs it as the run: tools get their
// connection tokens on the host, never here. The host records each call on the step, so the run's
// graph shows what the script did. The harness (`run.ts`) sets how calls travel, once, before it
// imports the script:
// - **relay** (a host running Deno next to itself, the OSS server): each call is one stderr line
//   and its answer one stdin line; the host makes the call. The script has no network access to
//   the host and never sees a token.
// - **http** (the cloud sandbox): `POST <url>/call` with the step's run token, a short-lived token
//   bound to this run and step that dies when the step ends. The token stays in a closure here.
// Without a transport (a host with no SDK) every call throws.

/** How a call reaches the host: `op` and its arguments in, the result out (throws on refusal). */
export type Transport = (op: string, args: Record<string, unknown>) => Promise<unknown>;

let transport: Transport | undefined;

/** Set how calls travel. The harness calls it once; a second call throws. */
export function setTransport(t: Transport): void {
  if (transport) throw new Error("@distri/runtime: the transport is already set");
  transport = t;
}

/** A call the host refused or that failed (`code`: `unauthorized`, `invalid`, `too_many_calls`, `failed`). */
export class DistriCallError extends Error {
  constructor(message: string, readonly code: string) {
    super(message);
    this.name = "DistriCallError";
  }
}

/** The answer of a call: `{ok: true, result}` or `{ok: false, error, code}`. */
export interface CallAnswer {
  ok?: boolean;
  result?: unknown;
  error?: string;
  code?: string;
}

/** Unwrap an answer, throwing a `DistriCallError` for a refusal. */
export function unwrap(op: string, body: CallAnswer, status = 200): unknown {
  if (status >= 400 || body.ok === false) {
    throw new DistriCallError(`distri.${op}: ${body.error ?? status}`, body.code ?? "failed");
  }
  return body.result;
}

/** The http transport: the URL and the run token live only in this closure. */
export function httpTransport(url: string, token: string): Transport {
  const endpoint = `${url.replace(/\/+$/, "")}/call`;
  return async (op, args) => {
    const res = await fetch(endpoint, {
      method: "POST",
      headers: { "authorization": `Bearer ${token}`, "content-type": "application/json" },
      body: JSON.stringify({ op, ...args }),
    });
    const text = await res.text();
    let body: CallAnswer;
    try {
      body = JSON.parse(text);
    } catch {
      throw new DistriCallError(`distri.${op}: the host answered ${res.status}: ${text.slice(0, 200)}`, "failed");
    }
    return unwrap(op, body, res.status);
  };
}

function call(op: string, args: Record<string, unknown> = {}): Promise<unknown> {
  if (!transport) {
    return Promise.reject(new Error("@distri/runtime is not available here: this host offers no script SDK"));
  }
  return transport(op, args);
}

/** A chat button: a callback (goes to the workflow's callback entry points) or a link. */
export type Button =
  | { kind: "callback"; label: string; callback_data: string }
  | { kind: "url"; label: string; url: string };

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
