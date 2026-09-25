#!/usr/bin/env python3
"""A scripted model server for testing real agents without a model.

Speaks enough of the Anthropic Messages API (`/v1/messages`), of OpenAI
Chat Completions (`/v1/chat/completions`; streaming and not, for both) and
of OpenAI Responses (`/v1/responses`, streamed) for Claude Code, Codex-style
clients and Kitsu's own loop, records every request as one JSON line, and
plays either a script file (`--script`, all three APIs) or the built-in
script (Messages and Chat Completions):

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
  {"status": 429, "retry_after": 0}                         an HTTP error (429, 529, 500, 401...)
  {"stream_error": true}                                    a 200 whose stream fails (Messages: overloaded_error, Responses: response.failed)
  {"expect": "substring"}  (on any entry) the request must contain it, or 500
  {"reject": "substring"}  (on any entry) the request must NOT contain it, or 500
Chat Completions only:
  {"raw_tools": [{"tool": ..., "arguments": "<verbatim>", "id": null, "index": N}]}
                                                            calls sent as given (arguments
                                                            unparsed, id omitted when null)
  {"empty": true}                                           a reply with no text and no call
  {"finish": "length"}  (on any entry) the finish_reason sent instead of stop/tool_calls
Optional "usage": {"input": N, "output": M, "cached": C, "cache_write": W} on
any entry ("input" is what wasn't read from the cache). Past the end of the
script every request gets {"text": "script finished"}.

Every Responses reply starts with a reasoning item whose encrypted_content is
"enc-<n>", n counting the script's entries, so a test can see it come back.
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
    """The next script entry as ('tools', [(name, args)], usage, entry) / ('text', str, usage, entry) /
    ('raw', [call], usage, entry) / ('empty', None, usage, entry) / ('overflow',) / ('error', msg)."""
    with LOCK:
        i = STATE["step"]
        STATE["step"] += 1
    if i >= len(SCRIPT):
        return ("text", "script finished", {}, {})
    e = SCRIPT[i]
    if "expect" in e and e["expect"] not in text:
        return ("error", f"script step {i}: request does not contain {e['expect']!r}")
    if "reject" in e and e["reject"] in text:
        return ("error", f"script step {i}: request contains {e['reject']!r}")
    usage = e.get("usage", {})
    if e.get("overflow"):
        return ("overflow",)
    if e.get("stream_error"):
        return ("stream_error",)
    if "status" in e:
        return ("http", e["status"], e.get("retry_after"))
    if "tool" in e:
        return ("tools", [(e["tool"], e.get("args", {}))], usage, e)
    if "tools" in e:
        return ("tools", [(t["tool"], t.get("args", {})) for t in e["tools"]], usage, e)
    if "raw_tools" in e:
        return ("raw", e["raw_tools"], usage, e)
    if e.get("empty"):
        return ("empty", None, usage, e)
    return ("text", e.get("text", ""), usage, e)


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


def anthropic_text(body):
    out = [text_of(body.get("system")) if isinstance(body.get("system"), list) else str(body.get("system") or "")]
    for m in body.get("messages", []):
        c = m.get("content")
        out.append(text_of(c))
        for b in c if isinstance(c, list) else []:
            if isinstance(b, dict) and b.get("type") == "tool_use":
                out.append(json.dumps(b.get("input")))
    return "\n".join(out)


def responses_text(body):
    out = []
    for item in body.get("input") or []:
        c = item.get("content")
        if isinstance(c, str):
            out.append(c)
        elif isinstance(c, list):
            out.extend(p.get("text", "") for p in c if isinstance(p, dict))
        for k in ("name", "arguments", "output", "encrypted_content"):
            if isinstance(item.get(k), str):
                out.append(item[k])
    return "\n".join(out)


ANTHROPIC_ERRORS = {400: "invalid_request_error", 401: "authentication_error", 403: "permission_error",
                    429: "rate_limit_error", 500: "api_error", 529: "overloaded_error"}


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
        with LOCK, open(self.args.log, "a", encoding="utf-8", newline="\n") as f:
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
        if reply[0] == "http":
            self.log(path, f"http {reply[1]}", body)
            raw = json.dumps({"error": {"message": f"scripted HTTP {reply[1]}", "type": "stub"}}).encode()
            self.send_response(reply[1])
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(raw)))
            if reply[2] is not None:
                self.send_header("retry-after", str(reply[2]))
            self.end_headers()
            self.wfile.write(raw)
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
            calls = [{"index": k, "id": f"call_{mid}_{k}", "type": "function",
                      "function": {"name": name, "arguments": json.dumps(a)}} for k, (name, a) in enumerate(reply[1])]
            message = {"role": "assistant", "content": None, "tool_calls": calls}
            finish = "tool_calls"
        elif reply[0] == "raw":
            calls = []
            for k, t in enumerate(reply[1]):
                c = {"index": t.get("index", k), "type": "function",
                     "function": {"name": t.get("tool", ""), "arguments": t.get("arguments", "")}}
                if t.get("id", f"call_{mid}_{k}") is not None:
                    c["id"] = t.get("id", f"call_{mid}_{k}")
                calls.append(c)
            message = {"role": "assistant", "content": None, "tool_calls": calls}
            finish = "tool_calls"
        elif reply[0] == "empty":
            message = {"role": "assistant", "content": None}
            finish = "stop"
        else:
            message = {"role": "assistant", "content": reply[1]}
            finish = "stop"
        entry = reply[3] if len(reply) > 3 else {}
        finish = entry.get("finish", finish)
        if not body.get("stream"):
            if message.get("tool_calls"):
                message = dict(message, tool_calls=[{k: v for k, v in tc.items() if k != "index"} for tc in message["tool_calls"]])
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
        if message.get("tool_calls"):
            for tc in message["tool_calls"]:
                first = {"index": tc["index"], "type": "function", "function": {"name": tc["function"]["name"], "arguments": ""}}
                if "id" in tc:
                    first["id"] = tc["id"]
                chunk({"tool_calls": [first]})
                chunk({"tool_calls": [{"index": tc["index"], "function": {"arguments": tc["function"]["arguments"]}}]})
        elif message["content"] is not None:
            chunk({"content": message["content"]})
        chunk({}, finish)
        if (body.get("stream_options") or {}).get("include_usage"):
            c = {"id": mid, "object": "chat.completion.chunk", "created": int(time.time()), "model": model, "choices": [], "usage": u}
            self.wfile.write(f"data: {json.dumps(c)}\n\n".encode())
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()

    def http_error(self, path, body, status, retry_after, err):
        self.log(path, f"http {status}", body)
        raw = json.dumps(err).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(raw)))
        if retry_after is not None:
            self.send_header("retry-after", str(retry_after))
        self.end_headers()
        self.wfile.write(raw)

    def stream_start(self):
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.end_headers()

    def auth(self):
        # Which scheme the key came in, never the key.
        a = self.headers.get("authorization") or ""
        return {"authorization": a.split(" ")[0] if a else None, "x-api-key": self.headers.get("x-api-key") is not None,
                "anthropic-version": self.headers.get("anthropic-version")}

    def anthropic(self, path, body):
        """Scripted Messages API, streamed."""
        reply = scripted(anthropic_text(body))
        if reply[0] == "error":
            self.log(path, "error", body)
            self._json(500, {"type": "error", "error": {"type": "api_error", "message": reply[1]}})
            return
        if reply[0] == "http":
            kind = ANTHROPIC_ERRORS.get(reply[1], "api_error")
            self.http_error(path, body, reply[1], reply[2],
                            {"type": "error", "error": {"type": kind, "message": f"scripted HTTP {reply[1]}"}})
            return
        if reply[0] == "overflow":
            self.log(path, "overflow", body)
            self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                                         "message": "prompt is too long: 210000 tokens > 200000 maximum"}})
            return
        self.log(path, reply[0], dict(body, _headers=self.auth()))
        mid = f"msg_{int(time.time() * 1000)}"
        self.stream_start()
        if reply[0] == "stream_error":
            sse(self, "message_start", {"type": "message_start", "message": {"id": mid, "type": "message", "role": "assistant",
                                                                               "content": [], "usage": {"input_tokens": 10, "output_tokens": 1}}})
            sse(self, "error", {"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}})
            return
        usage = reply[2]
        sse(self, "message_start", {"type": "message_start", "message": {
            "id": mid, "type": "message", "role": "assistant", "model": body.get("model", "stub"), "content": [],
            "stop_reason": None, "stop_sequence": None,
            "usage": {"input_tokens": usage.get("input", 1000), "output_tokens": 1,
                      "cache_read_input_tokens": usage.get("cached", 0), "cache_creation_input_tokens": usage.get("cache_write", 0)}}})
        sse(self, "ping", {"type": "ping"})
        if reply[0] == "tools":
            for k, (name, a) in enumerate(reply[1]):
                sse(self, "content_block_start", {"type": "content_block_start", "index": k, "content_block": {
                    "type": "tool_use", "id": f"toolu_{mid}_{k}", "name": name, "input": {}}})
                raw = json.dumps(a)
                for part in (raw[: len(raw) // 2], raw[len(raw) // 2:]):
                    sse(self, "content_block_delta", {"type": "content_block_delta", "index": k,
                                                      "delta": {"type": "input_json_delta", "partial_json": part}})
                sse(self, "content_block_stop", {"type": "content_block_stop", "index": k})
            stop = "tool_use"
        else:
            sse(self, "content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})
            sse(self, "content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": reply[1]}})
            sse(self, "content_block_stop", {"type": "content_block_stop", "index": 0})
            stop = "end_turn"
        sse(self, "message_delta", {"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": None},
                                    "usage": {"output_tokens": usage.get("output", 20)}})
        sse(self, "message_stop", {"type": "message_stop"})

    def openai_responses(self, path, body):
        """Scripted Responses API, streamed, stateless."""
        with LOCK:
            n = STATE["step"]
        reply = scripted(responses_text(body))
        if reply[0] == "error":
            self.log(path, "error", body)
            self._json(500, {"error": {"message": reply[1], "type": "server_error", "code": "server_error"}})
            return
        if reply[0] == "http":
            self.http_error(path, body, reply[1], reply[2],
                            {"error": {"message": f"scripted HTTP {reply[1]}", "type": "stub", "code": None}})
            return
        if reply[0] == "overflow":
            self.log(path, "overflow", body)
            self._json(400, {"error": {"message": "Your input exceeds the context window of this model. Please adjust your input and try again.",
                                       "type": "invalid_request_error", "param": "input", "code": "context_length_exceeded"}})
            return
        self.log(path, reply[0], dict(body, _headers=self.auth()))
        rid = f"resp_{int(time.time() * 1000)}"
        seq = iter(range(1000))
        def ev(data):
            data["sequence_number"] = next(seq)
            sse(self, data["type"], data)
        self.stream_start()
        ev({"type": "response.created", "response": {"id": rid, "status": "in_progress", "output": []}})
        if reply[0] == "stream_error":
            ev({"type": "response.failed", "response": {"id": rid, "status": "failed", "output": [],
                                                        "error": {"code": "server_error", "message": "The model failed to generate a response."}}})
            return
        output = [{"type": "reasoning", "id": f"rs_{rid}", "summary": [], "encrypted_content": f"enc-{n}"}]
        if reply[0] == "tools":
            for k, (name, a) in enumerate(reply[1]):
                output.append({"type": "function_call", "id": f"fc_{rid}_{k}", "call_id": f"call_{rid}_{k}",
                               "name": name, "arguments": json.dumps(a), "status": "completed"})
        else:
            output.append({"type": "message", "id": f"msg_{rid}", "role": "assistant", "status": "completed",
                           "content": [{"type": "output_text", "text": reply[1], "annotations": []}]})
        for i, item in enumerate(output):
            ev({"type": "response.output_item.added", "output_index": i, "item": dict(item, status="in_progress")})
            if item["type"] == "function_call":
                ev({"type": "response.function_call_arguments.delta", "output_index": i, "item_id": item["id"], "delta": item["arguments"]})
            if item["type"] == "message":
                ev({"type": "response.output_text.delta", "output_index": i, "item_id": item["id"], "content_index": 0,
                    "delta": item["content"][0]["text"]})
            ev({"type": "response.output_item.done", "output_index": i, "item": item})
        usage = reply[2]
        cached = usage.get("cached", 0)
        ev({"type": "response.completed", "response": {
            "id": rid, "object": "response", "status": "completed", "model": body.get("model", "stub"), "output": output,
            "usage": {"input_tokens": usage.get("input", 1000) + cached, "output_tokens": usage.get("output", 20),
                      "input_tokens_details": {"cached_tokens": cached}, "output_tokens_details": {"reasoning_tokens": 5}}}})

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
        if path.endswith("/responses"):
            self.openai_responses(path, body)
            return
        if path.endswith("/messages") and SCRIPT is not None:
            self.anthropic(path, body)
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
                with LOCK, open(self.args.log, "a", encoding="utf-8", newline="\n") as f:
                    f.write(json.dumps({"at": time.time(), "path": path, "role": "overflow", "body": body}) + "\n")
                self._json(400, {"type": "error", "error": {"type": "invalid_request_error",
                                                             "message": "prompt is too long: 210000 tokens > 200000 maximum"}})
                return
        kind, *rest = plan(body, self.args)
        role = rest[-1]
        with LOCK, open(self.args.log, "a", encoding="utf-8", newline="\n") as f:
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
        with open(args.script, encoding="utf-8") as f:
            SCRIPT = json.load(f)
    Handler.args = args
    srv = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    # --port 0 picks a free port; this line says which.
    print(f"stub model on 127.0.0.1:{srv.server_address[1]}", file=sys.stderr, flush=True)
    srv.serve_forever()


if __name__ == "__main__":
    main()
