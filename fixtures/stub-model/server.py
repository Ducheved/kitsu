#!/usr/bin/env python3
"""A scripted model server for testing real agents without a model.

Speaks enough of the Anthropic Messages API and of OpenAI Chat Completions
(`/v1/chat/completions`; streaming and not, for both) for Claude Code,
Codex-style clients and Kitsu's own loop, records every request as one JSON
line, and plays either a script file (`--script`) or the built-in script:

  1. answer the first few turns with a tool call (read a file), reporting a
     large `input_tokens` so the agent believes its context is nearly full;
  2. when a request looks like a compaction request (the agent asking for a
     summary of the conversation), answer with a short summary;
  3. after that, finish the turn with plain text.

No model, no network, no API key. Used by the compaction probe to see what
an agent keeps of Kitsu's brief after it compacts.

    python3 server.py --port 8791 --log requests.jsonl [--fill 185000] [--tool-turns 3]
    python3 server.py --port 8791 --log requests.jsonl --script steps.json

A script is a JSON list, one entry per model request, played in order:
  {"tool": "read_file", "args": {"path": "payments.py"}}   a tool call
  {"tools": [{"tool": ..., "args": ...}, ...]}              several at once
  {"text": "Done."}                                         a final answer
  {"overflow": true}                                        a context-overflow error
  {"expect": "substring"}  (on any entry) the request must contain it, or 500
Optional "usage": {"input": N, "output": M} on any entry. Past the end of the
script every request gets {"text": "script finished"}.
"""

import argparse
import json
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOCK = threading.Lock()
STATE = {"turns": 0, "compactions": 0, "step": 0}
SCRIPT = None

SUMMARY_MARKERS = (
    "summary of the conversation",
    "summarize the conversation",
    "create a detailed summary",
    "your task is to create a detailed summary",
    "compact",
)


def text_of(content):
    if isinstance(content, str):
        return content
    out = []
    for block in content or []:
        if isinstance(block, dict):
            if block.get("type") == "text":
                out.append(block.get("text", ""))
            elif block.get("type") == "tool_result":
                out.append(text_of(block.get("content")))
    return "\n".join(out)


def request_text(body):
    parts = [text_of(body.get("system")) if isinstance(body.get("system"), list) else str(body.get("system") or "")]
    for m in body.get("messages", []):
        parts.append(text_of(m.get("content")))
    return "\n".join(parts)


def is_compaction(body):
    # Only look at the last user message: earlier turns may quote anything.
    msgs = body.get("messages") or []
    last = text_of(msgs[-1].get("content")) if msgs else ""
    low = last.lower()
    return any(m in low for m in SUMMARY_MARKERS) and "summary" in low


def openai_text(body):
    out = []
    for m in body.get("messages", []):
        c = m.get("content")
        if isinstance(c, str):
            out.append(c)
        elif isinstance(c, list):
            out.extend(b.get("text", "") for b in c if isinstance(b, dict))
        for tc in m.get("tool_calls") or []:
            out.append(json.dumps(tc))
    return "\n".join(out)


def scripted(text):
    """The next script entry as ('tools', [(name, args)], usage) / ('text', str, usage) / ('overflow',) / ('error', msg)."""
    with LOCK:
        i = STATE["step"]
        STATE["step"] += 1
    if i >= len(SCRIPT):
        return ("text", "script finished", {})
    e = SCRIPT[i]
    if "expect" in e and e["expect"] not in text:
        return ("error", f"script step {i}: request does not contain {e['expect']!r}")
    usage = e.get("usage", {})
    if e.get("overflow"):
        return ("overflow",)
    if "tool" in e:
        return ("tools", [(e["tool"], e.get("args", {}))], usage)
    if "tools" in e:
        return ("tools", [(t["tool"], t.get("args", {})) for t in e["tools"]], usage)
    return ("text", e.get("text", ""), usage)


def plan(body, args):
    """Decide the reply: ('tool', name, input) or ('text', str)."""
    with LOCK:
        if is_compaction(body):
            STATE["compactions"] += 1
            return ("text", "Summary: the user asked to fix the retry loop in payments.py. "
                    "Work so far: read payments.py. Next: change the loop.", "compaction")
        STATE["turns"] += 1
        turn = STATE["turns"]
    if turn <= args.tool_turns:
        return ("tool", args.tool, {"file_path": args.read} if args.tool == "Read" else {"command": "ls"}, "work")
    return ("text", "Done looking. Stopping here.", "final")


def sse(handler, event, data):
    handler.wfile.write(f"event: {event}\ndata: {json.dumps(data)}\n\n".encode())
    handler.wfile.flush()


class Handler(BaseHTTPRequestHandler):
    args = None

    def log_message(self, *a):
        pass

    def _json(self, code, obj):
        raw = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self):
        self._json(200, {"data": [], "has_more": False})

    def log(self, path, role, body):
        with LOCK, open(self.args.log, "a") as f:
            f.write(json.dumps({"at": time.time(), "path": path, "role": role, "body": body}) + "\n")

    def openai(self, path, body):
        text = openai_text(body)
        if SCRIPT is not None:
            reply = scripted(text)
        else:
            kind, *rest = plan({"messages": [{"content": text}]}, self.args)
            reply = ("tools", [(rest[0], rest[1])], {}) if kind == "tool" else ("text", rest[0], {})
        if reply[0] == "error":
            self.log(path, "error", body)
            self._json(500, {"error": {"message": reply[1], "type": "script_error"}})
            return
        if reply[0] == "overflow":
            self.log(path, "overflow", body)
            self._json(400, {"error": {"message": "This model's maximum context length is 128000 tokens. However, your messages resulted in 131072 tokens.",
                                       "type": "invalid_request_error", "code": "context_length_exceeded"}})
            return
        self.log(path, reply[0], body)
        usage = reply[2] if len(reply) > 2 else {}
        u = {"prompt_tokens": usage.get("input", 1000), "completion_tokens": usage.get("output", 20)}
        u["total_tokens"] = u["prompt_tokens"] + u["completion_tokens"]
        mid = f"chatcmpl-{int(time.time() * 1000)}"
        model = body.get("model", "stub")
        if reply[0] == "tools":
            calls = [{"id": f"call_{mid}_{k}", "type": "function",
                      "function": {"name": name, "arguments": json.dumps(a)}} for k, (name, a) in enumerate(reply[1])]
            message = {"role": "assistant", "content": None, "tool_calls": calls}
            finish = "tool_calls"
        else:
            message = {"role": "assistant", "content": reply[1]}
            finish = "stop"
        if not body.get("stream"):
            self._json(200, {"id": mid, "object": "chat.completion", "created": int(time.time()), "model": model,
                             "choices": [{"index": 0, "message": message, "finish_reason": finish}], "usage": u})
            return
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        def chunk(delta, finish_reason=None, usage=None):
            c = {"id": mid, "object": "chat.completion.chunk", "created": int(time.time()), "model": model,
                 "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}]}
            if usage is not None:
                c["usage"] = usage
            self.wfile.write(f"data: {json.dumps(c)}\n\n".encode())
            self.wfile.flush()
        chunk({"role": "assistant"})
        if reply[0] == "tools":
            for k, tc in enumerate(message["tool_calls"]):
                chunk({"tool_calls": [{"index": k, "id": tc["id"], "type": "function",
                                       "function": {"name": tc["function"]["name"], "arguments": ""}}]})
                chunk({"tool_calls": [{"index": k, "function": {"arguments": tc["function"]["arguments"]}}]})
        else:
            chunk({"content": message["content"]})
        chunk({}, finish)
        if (body.get("stream_options") or {}).get("include_usage"):
            c = {"id": mid, "object": "chat.completion.chunk", "created": int(time.time()), "model": model, "choices": [], "usage": u}
            self.wfile.write(f"data: {json.dumps(c)}\n\n".encode())
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()

    def do_POST(self):
        n = int(self.headers.get("content-length") or 0)
        try:
            body = json.loads(self.rfile.read(n) or b"{}")
        except ValueError:
            body = {}
        path = self.path.split("?")[0]
        if path.endswith("/chat/completions"):
            self.openai(path, body)
            return
        if path.endswith("/count_tokens"):
            self._json(200, {"input_tokens": self.args.fill})
            return
        if not path.endswith("/messages"):
            self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": path}})
            return
        if self.args.overflow_at:
            with LOCK:
                STATE["seen"] = STATE.get("seen", 0) + 1
                overflow = STATE["seen"] == self.args.overflow_at
            if overflow:
                with LOCK, open(self.args.log, "a") as f:
                    f.write(json.dumps({"at": time.time(), "path": path, "role": "overflow", "body": body}) + "\n")
                self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                                             "message": "prompt is too long: 210000 tokens > 200000 maximum"}})
                return
        kind, *rest = plan(body, self.args)
        role = rest[-1]
        with LOCK, open(self.args.log, "a") as f:
            f.write(json.dumps({"at": time.time(), "path": path, "role": role, "body": body}) + "\n")
        model = body.get("model", "stub")
        usage_in = self.args.fill if role == "work" else 2000
        mid = f"msg_{int(time.time() * 1000)}"
        if kind == "tool":
            name, tool_input = rest[0], rest[1]
            block = {"type": "tool_use", "id": f"toolu_{mid}", "name": name, "input": {}}
            stop = "tool_use"
            deltas = [{"type": "input_json_delta", "partial_json": json.dumps(tool_input)}]
            final_block = dict(block, input=tool_input)
        else:
            text = rest[0]
            block = {"type": "text", "text": ""}
            stop = "end_turn"
            deltas = [{"type": "text_delta", "text": text}]
            final_block = {"type": "text", "text": text}
        usage = {"input_tokens": usage_in, "output_tokens": 20, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}
        if not body.get("stream"):
            self._json(200, {"id": mid, "type": "message", "role": "assistant", "model": model,
                             "content": [final_block], "stop_reason": stop, "stop_sequence": None, "usage": usage})
            return
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.end_headers()
        sse(self, "message_start", {"type": "message_start", "message": {
            "id": mid, "type": "message", "role": "assistant", "model": model, "content": [],
            "stop_reason": None, "stop_sequence": None, "usage": usage}})
        sse(self, "content_block_start", {"type": "content_block_start", "index": 0, "content_block": block})
        for d in deltas:
            sse(self, "content_block_delta", {"type": "content_block_delta", "index": 0, "delta": d})
        sse(self, "content_block_stop", {"type": "content_block_stop", "index": 0})
        sse(self, "message_delta", {"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": None},
                                    "usage": {"output_tokens": 20}})
        sse(self, "message_stop", {"type": "message_stop"})


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--port", type=int, default=8791)
    p.add_argument("--log", required=True)
    p.add_argument("--fill", type=int, default=185000, help="input_tokens reported on work turns")
    p.add_argument("--tool-turns", type=int, default=3)
    p.add_argument("--tool", default="Read")
    p.add_argument("--read", default="payments.py")
    p.add_argument("--overflow-at", type=int, default=0,
                   help="answer the Nth messages request with a context-overflow error, once")
    p.add_argument("--script", help="JSON list of replies to play in order (see the module docstring)")
    args = p.parse_args()
    global SCRIPT
    if args.script:
        with open(args.script) as f:
            SCRIPT = json.load(f)
    Handler.args = args
    srv = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"stub model on 127.0.0.1:{args.port}", file=sys.stderr, flush=True)
    srv.serve_forever()


if __name__ == "__main__":
    main()
