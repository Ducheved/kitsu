#!/usr/bin/env python3
"""A scripted model server for testing real agents without a model.

Speaks enough of the Anthropic Messages API (streaming and not) for Claude
Code, records every request as one JSON line, and plays a fixed script:

  1. answer the first few turns with a tool call (read a file), reporting a
     large `input_tokens` so the agent believes its context is nearly full;
  2. when a request looks like a compaction request (the agent asking for a
     summary of the conversation), answer with a short summary;
  3. after that, finish the turn with plain text.

No model, no network, no API key. Used by the compaction probe to see what
an agent keeps of Kitsu's brief after it compacts.

    python3 server.py --port 8791 --log requests.jsonl [--fill 185000] [--tool-turns 3]
"""

import argparse
import json
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOCK = threading.Lock()
STATE = {"turns": 0, "compactions": 0}

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

    def do_POST(self):
        n = int(self.headers.get("content-length") or 0)
        try:
            body = json.loads(self.rfile.read(n) or b"{}")
        except ValueError:
            body = {}
        path = self.path.split("?")[0]
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
    args = p.parse_args()
    Handler.args = args
    srv = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"stub model on 127.0.0.1:{args.port}", file=sys.stderr, flush=True)
    srv.serve_forever()


if __name__ == "__main__":
    main()
