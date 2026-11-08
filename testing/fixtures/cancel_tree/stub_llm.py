#!/usr/bin/env python3
"""Scripted OpenAI-compatible chat completions for the cancel-tree fixture.

- A request offering `invoke_agent` is cancel_parent: with no tool result yet
  it calls `invoke_agent` on cancel_slow (`mode: background` when the user
  text says "background"), then `final`.
- Any other request is cancel_slow: it answers `final` after SLOW_SECS
  (default 300), so the child stays working long enough to cancel.

Usage: python3 -I stub_llm.py [port]   (default 18931)
"""
import json
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SLOW_SECS = float(os.environ.get("SLOW_SECS", "300"))


def text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return " ".join(p.get("text", "") for p in content if isinstance(p, dict))
    return ""


def decide(req):
    tools = [t.get("function", {}).get("name") for t in req.get("tools") or []]
    msgs = req.get("messages") or []
    if "invoke_agent" in tools:
        if any(m.get("role") == "tool" for m in msgs):
            return "final", {"input": "cancel_slow was started."}
        user = " ".join(text_of(m.get("content")) for m in msgs if m.get("role") == "user")
        args = {"agent": "cancel_slow", "prompt": "Take your time.", "title": "Slow child"}
        if "background" in user.lower():
            args["mode"] = "background"
        return "invoke_agent", args
    time.sleep(SLOW_SECS)
    return "final", {"input": "slow child finished."}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        sys.stderr.write("stub: " + (fmt % args) + "\n")

    def do_GET(self):
        body = json.dumps({"object": "list", "data": [{"id": "stub", "object": "model"}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        name, args = decide(req)
        call = {"id": f"call_{int(time.time() * 1000)}", "type": "function",
                "function": {"name": name, "arguments": json.dumps(args)}}
        usage = {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        model = req.get("model", "stub")
        try:
            if req.get("stream"):
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                chunks = [
                    {"choices": [{"index": 0, "delta": {"role": "assistant", "content": None,
                                  "tool_calls": [dict(call, index=0)]}, "finish_reason": None}]},
                    {"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]},
                    {"choices": [], "usage": usage},
                ]
                for c in chunks:
                    c.update(id="chatcmpl-stub", object="chat.completion.chunk", created=int(time.time()), model=model)
                    self.wfile.write(f"data: {json.dumps(c)}\n\n".encode())
                self.wfile.write(b"data: [DONE]\n\n")
                self.wfile.flush()
            else:
                body = json.dumps({
                    "id": "chatcmpl-stub", "object": "chat.completion", "created": int(time.time()),
                    "model": model, "usage": usage,
                    "choices": [{"index": 0, "finish_reason": "tool_calls",
                                 "message": {"role": "assistant", "content": None, "tool_calls": [call]}}],
                }).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18931
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
