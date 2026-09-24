#!/usr/bin/env python3
"""Compaction probe: one Kitsu run of real Claude Code against the stub model.

Answers "does the agent still see Kitsu's rules after it compacts its own
context?" without a model or an API key. The stub (server.py) answers the
Nth request with a context-overflow error, which makes Claude Code compact;
then this prints, for every request, whether the invariant text, "Done
means" and the task title are in the system prompt (sys) or the messages
(msg).

Needs the Claude ACP adapter installed somewhere:
    npm install @agentclientprotocol/claude-agent-acp
and a built kitsu. Paths come from the environment:
    CLAUDE_ACP=<node_modules>/@agentclientprotocol/claude-agent-acp/dist/index.js
    KITSU=<repo>/target/debug/kitsu   PROBE_DIR=<scratch dir>

    python3 fixtures/stub-model/probe_claude.py --overflow-at 3 --fill 20000

Kitsu runs with a clean environment (env -i): an agent inherits Kitsu's
environment, and a host's tokens and settings would otherwise leak into it
and change its behaviour. IS_SANDBOX=1 lets Claude Code start as root in a
disposable container; it doesn't change the permission mode.
"""
import argparse, json, os, shutil, subprocess, sys, time
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "../.."))
S = os.environ.get("PROBE_DIR", "/tmp/kitsu-probe")
K = os.environ.get("KITSU", os.path.join(ROOT, "target/debug/kitsu"))
ADAPTER = os.environ["CLAUDE_ACP"]
p = argparse.ArgumentParser(); p.add_argument("--append"); p.add_argument("--port", type=int, default=8791); p.add_argument("--tag", default="run"); p.add_argument("--fill", default="185000"); p.add_argument("--turns", default="3"); p.add_argument("--overflow-at", default="0"); p.add_argument("--env", action="append", default=[])
a = p.parse_args()
W = f"{S}/{a.tag}"; shutil.rmtree(W, ignore_errors=True); os.makedirs(f"{W}/cfg"); os.makedirs(f"{W}/home")
shutil.copytree(os.path.join(ROOT, "fixtures/retry-storm/repo"), f"{W}/repo")
g = lambda *x: subprocess.run(["git", "-c", "user.name=Dev", "-c", "user.email=d@e", *x], cwd=f"{W}/repo", check=True, capture_output=True)
g("init", "-q", "-b", "main"); g("add", "-A"); g("commit", "-qm", "init")
stub = subprocess.Popen(["python3", os.path.join(HERE, "server.py"), "--port", str(a.port), "--log", f"{W}/requests.jsonl", "--fill", a.fill, "--tool-turns", a.turns, "--overflow-at", a.overflow_at], stderr=open(f"{W}/stub.err", "w"))
time.sleep(0.5)
sp = {"excludeDynamicSections": True}
if a.append: sp["append"] = a.append
env = {"ANTHROPIC_BASE_URL": f"http://127.0.0.1:{a.port}", "ANTHROPIC_API_KEY": "stub", "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
       "DISABLE_AUTOUPDATER": "1", "HOME": f"{W}/home", "ANTHROPIC_MODEL": "claude-sonnet-4-5", "IS_SANDBOX": "1"}
for kv in a.env: k, v = kv.split("=", 1); env[k] = v
q = lambda s: json.dumps(s)
toml = "[agents.claude-stub]\ncommand = [\"node\", %s]\nenv = { %s }\nmeta = { systemPrompt = { %s } }\n" % (
    q(ADAPTER),
    ", ".join(f"{k} = {q(v)}" for k, v in env.items()),
    ", ".join(f"{k} = {q(v) if isinstance(v, str) else str(v).lower()}" for k, v in sp.items()))
open(f"{W}/cfg/agents.toml", "w").write(toml)
clean = {"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": f"{W}/home", "KITSU_CONFIG_DIR": f"{W}/cfg"}
subprocess.run([K, "trust"], cwd=f"{W}/repo", env=clean, capture_output=True)
r = subprocess.run([K, "run", "bounded-retries", "--agent", "claude-stub", "--policy", "auto", "--id", "rprobe", "-q"], cwd=f"{W}/repo", env=clean, capture_output=True, text=True, timeout=300)
stub.terminate()
print("kitsu run exit", r.returncode, r.stderr.strip().splitlines()[-1:] )
def txt(c):
    if isinstance(c, str): return c
    return "\n".join((b.get("text") or "") if b.get("type") != "tool_result" else txt(b.get("content")) for b in c or [])
marks = {"invariant": "One idempotency key per logical charge", "done-means": "Done means", "task-title": "Stop retrying forever", "appended": (a.append or "\0")[:40]}
for i, l in enumerate(open(f"{W}/requests.jsonl")):
    rq = json.loads(l); b = rq["body"]
    sys_t = txt(b.get("system")) if isinstance(b.get("system"), list) else str(b.get("system") or "")
    msgs = "\n".join(txt(m["content"]) for m in b.get("messages", []))
    print(i + 1, rq["role"], {k: ("sys" if v in sys_t else "") + ("msg" if v in msgs else "") or "-" for k, v in marks.items()}, )
